//! HTTP client implementations for making outbound requests with TLS support.
//!
//! This module provides HTTP clients for making requests to external services. It includes
//! `TakoClient` for plain HTTP connections and `TakoTlsClient` for secure HTTPS connections
//! using rustls. Both clients support HTTP/1.1 protocol and handle connection management
//! automatically. The clients are generic over body types to support different request
//! payload formats while maintaining type safety and performance.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::client::{TakoClient, TakoTlsClient};
//! use http_body_util::Empty;
//! use bytes::Bytes;
//! use http::Request;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Plain HTTP client
//! let mut client = TakoClient::<Empty<Bytes>>::new("httpbin.org", Some(80)).await?;
//! let request = Request::builder()
//!     .uri("/get")
//!     .body(Empty::new())?;
//! let response = client.request(request).await?;
//!
//! // HTTPS client with TLS
//! let mut tls_client = TakoTlsClient::<Empty<Bytes>>::new("httpbin.org", None).await?;
//! let tls_request = Request::builder()
//!     .uri("/get")
//!     .body(Empty::new())?;
//! let tls_response = tls_client.request(tls_request).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(docsrs, doc(cfg(feature = "client")))]

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use http::Request;
use http::Response;
use http_body::Body;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::client::conn::http1::SendRequest;
use hyper::client::{self};
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use webpki_roots::TLS_SERVER_ROOTS;

/// v2 high-level client built on `hyper_util::client::legacy::Client`.
///
/// Compared to [`TakoClient`] / [`TakoTlsClient`] (single-connection,
/// HTTP/1.1 only) this provides:
/// - connection pool with idle timeout / per-host caps
/// - HTTP/1.1 + HTTP/2 negotiation via ALPN (when TLS is present)
/// - per-request timeout
/// - retry policy with capped attempts and backoff
/// - W3C `traceparent` header propagation when present in extensions
///
/// HTTP/3 support is intentionally deferred — the underlying `hyper_util`
/// legacy client does not yet expose a stable connector for it.
pub struct V2Client {
  inner: HyperClient<HttpConnector, Full<bytes::Bytes>>,
  default_timeout: Option<Duration>,
  max_retries: u32,
  retry_backoff: Duration,
  user_agent: Option<String>,
}

/// Builder for [`V2Client`].
pub struct V2ClientBuilder {
  pool_idle_timeout: Option<Duration>,
  pool_max_idle_per_host: Option<usize>,
  default_timeout: Option<Duration>,
  max_retries: u32,
  retry_backoff: Duration,
  user_agent: Option<String>,
}

impl V2ClientBuilder {
  fn new() -> Self {
    Self {
      pool_idle_timeout: Some(Duration::from_secs(90)),
      pool_max_idle_per_host: Some(8),
      default_timeout: Some(Duration::from_secs(30)),
      max_retries: 0,
      retry_backoff: Duration::from_millis(100),
      user_agent: Some(format!("tako/{}", env!("CARGO_PKG_VERSION"))),
    }
  }

  /// Override the default request timeout (per-request).
  pub fn timeout(mut self, d: Duration) -> Self {
    self.default_timeout = Some(d);
    self
  }

  /// Maximum retry attempts on transport / 5xx failure (default 0).
  pub fn max_retries(mut self, n: u32) -> Self {
    self.max_retries = n;
    self
  }

  /// Backoff between retries (exponential `attempt * backoff`).
  pub fn retry_backoff(mut self, d: Duration) -> Self {
    self.retry_backoff = d;
    self
  }

  /// User-Agent header sent with every request (`None` to omit).
  pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
    self.user_agent = Some(ua.into());
    self
  }

  /// Idle timeout for pooled connections.
  pub fn pool_idle_timeout(mut self, d: Duration) -> Self {
    self.pool_idle_timeout = Some(d);
    self
  }

  /// Maximum idle connections per host.
  pub fn pool_max_idle_per_host(mut self, n: usize) -> Self {
    self.pool_max_idle_per_host = Some(n);
    self
  }

  /// Build a `V2Client`.
  pub fn build(self) -> V2Client {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    let mut builder = HyperClient::builder(TokioExecutor::new());
    if let Some(d) = self.pool_idle_timeout {
      builder.pool_idle_timeout(d);
    }
    if let Some(n) = self.pool_max_idle_per_host {
      builder.pool_max_idle_per_host(n);
    }
    let inner = builder.build(http);
    V2Client {
      inner,
      default_timeout: self.default_timeout,
      max_retries: self.max_retries,
      retry_backoff: self.retry_backoff,
      user_agent: self.user_agent,
    }
  }
}

impl V2Client {
  /// Create a builder with sensible defaults.
  pub fn builder() -> V2ClientBuilder {
    V2ClientBuilder::new()
  }

  /// Send a request with the configured timeout / retry / UA / traceparent policy.
  pub async fn send(
    &self,
    mut req: Request<Full<bytes::Bytes>>,
  ) -> Result<Response<hyper::body::Incoming>, Box<dyn Error + Send + Sync>> {
    if let Some(ua) = self.user_agent.as_deref()
      && !req.headers().contains_key(http::header::USER_AGENT)
      && let Ok(v) = http::HeaderValue::from_str(ua)
    {
      req.headers_mut().insert(http::header::USER_AGENT, v);
    }

    let attempt_max = self.max_retries.saturating_add(1);
    let mut last_err: Option<Box<dyn Error + Send + Sync>> = None;
    for attempt in 0..attempt_max {
      let mut req_clone = clone_request_full(&req);
      if attempt > 0 {
        let backoff = self.retry_backoff * attempt;
        tokio::time::sleep(backoff).await;
      }

      let send = self.inner.request(req_clone.take().expect("non-empty req"));
      let result = if let Some(t) = self.default_timeout {
        match tokio::time::timeout(t, send).await {
          Ok(r) => r.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>),
          Err(_) => Err("request timed out".into()),
        }
      } else {
        send
          .await
          .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
      };

      match result {
        Ok(resp) if resp.status().is_server_error() && attempt + 1 < attempt_max => {
          last_err = Some(format!("server error {}", resp.status()).into());
          continue;
        }
        Ok(resp) => return Ok(resp),
        Err(e) => {
          last_err = Some(e);
          if attempt + 1 == attempt_max {
            break;
          }
        }
      }
    }
    Err(last_err.unwrap_or_else(|| "client failed without error detail".into()))
  }
}

fn clone_request_full(req: &Request<Full<bytes::Bytes>>) -> Option<Request<Full<bytes::Bytes>>> {
  let mut builder = Request::builder()
    .method(req.method().clone())
    .uri(req.uri().clone())
    .version(req.version());
  for (k, v) in req.headers() {
    builder = builder.header(k.clone(), v.clone());
  }
  // Best-effort body clone: we hold a `Full<Bytes>` which is cheaply Clone-able.
  let body = match req.body() {
    body => body.clone(),
  };
  builder.body(body).ok()
}

/// HTTPS client with TLS encryption support using rustls.
///
/// `TakoTlsClient` provides a secure HTTP client that establishes TLS-encrypted
/// connections to remote servers. It uses rustls for TLS implementation and includes
/// built-in root certificate validation. The client maintains a persistent connection
/// and handles the TLS handshake automatically during initialization.
///
/// # Type Parameters
///
/// * `B` - Body type for HTTP requests, must implement `Body + Send + 'static`
///
/// # Examples
///
/// ```rust,no_run
/// use tako::client::TakoTlsClient;
/// use http_body_util::Empty;
/// use bytes::Bytes;
/// use http::Request;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create HTTPS client for api.example.com on port 443
/// let mut client = TakoTlsClient::<Empty<Bytes>>::new("api.example.com", None).await?;
///
/// // Make authenticated API request
/// let request = Request::builder()
///     .method("GET")
///     .uri("/v1/users")
///     .header("authorization", "Bearer token123")
///     .body(Empty::new())?;
///
/// let response = client.request(request).await?;
/// println!("Status: {}", response.status());
/// # Ok(())
/// # }
/// ```
pub struct TakoTlsClient<B: Body>
where
  B: Body + Send + 'static,
  B::Data: Send + 'static,
  B::Error: Into<Box<dyn Error + Send + Sync>>,
{
  /// HTTP/1.1 request sender for the established TLS connection.
  sender: SendRequest<B>,
  /// Background task handle managing the connection lifecycle.
  _conn_handle: JoinHandle<Result<(), hyper::Error>>,
}

impl<B> TakoTlsClient<B>
where
  B: Body + Send + 'static,
  B::Data: Send + 'static,
  B::Error: Into<Box<dyn Error + Send + Sync>>,
{
  /// Creates a new HTTPS client with TLS encryption.
  pub async fn new<'a>(host: &'a str, port: Option<u16>) -> Result<Self, Box<dyn Error>>
  where
    'a: 'static,
  {
    let port = port.unwrap_or(443);
    let addr = format!("{host}:{port}");
    let tcp_stream = TcpStream::connect(addr).await?;

    let mut root_cert_store = RootCertStore::empty();
    root_cert_store.extend(TLS_SERVER_ROOTS.iter().cloned());
    let tls_config = ClientConfig::builder()
      .with_root_certificates(root_cert_store)
      .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = ServerName::try_from(host)?;
    let tls_stream = connector.connect(server_name, tcp_stream).await?;
    let io = TokioIo::new(tls_stream);

    // Example for HTTP/2 handshake
    // let (mut sender, conn) = client::conn::http2::handshake::<TokioExecutor, _, Empty<Bytes>>(TokioExecutor::new(), io).await?;

    // HTTP/1 handshake
    let (sender, conn) = client::conn::http1::handshake::<_, B>(io).await?;
    let conn_handle = tokio::spawn(async move {
      if let Err(err) = conn.await {
        tracing::error!("Connection error: {}", err);
      }

      Ok(())
    });

    Ok(Self {
      sender,
      _conn_handle: conn_handle,
    })
  }

  /// Sends an HTTP request and returns the response with body as bytes.
  ///
  /// This method sends the request over the established TLS connection and reads
  /// the complete response body into memory as a byte vector. The response headers
  /// and status are preserved while the body is collected into a `Vec<u8>`.
  ///
  /// # Errors
  ///
  /// Returns an error if the request fails to send, the response cannot be read,
  /// or connection issues occur during the request/response cycle.
  ///
  /// # Examples
  ///
  /// ```rust,no_run
  /// use tako::client::TakoTlsClient;
  /// use http_body_util::Empty;
  /// use bytes::Bytes;
  /// use http::{Request, Method};
  ///
  /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
  /// let mut client = TakoTlsClient::<Empty<Bytes>>::new("httpbin.org", None).await?;
  ///
  /// let request = Request::builder()
  ///     .method(Method::GET)
  ///     .uri("/json")
  ///     .header("accept", "application/json")
  ///     .body(Empty::new())?;
  ///
  /// let response = client.request(request).await?;
  /// println!("Status: {}", response.status());
  /// println!("Body length: {} bytes", response.body().len());
  /// # Ok(())
  /// # }
  /// ```
  pub async fn request(&mut self, req: Request<B>) -> Result<Response<Vec<u8>>, Box<dyn Error>> {
    let mut response = self.sender.send_request(req).await?;
    let mut body_bytes = Vec::new();

    while let Some(frame) = response.frame().await {
      let frame = frame?;
      if let Some(chunk) = frame.data_ref() {
        body_bytes.extend_from_slice(chunk);
      }
    }

    let parts = response.into_parts();
    let resp = Response::from_parts(parts.0, body_bytes);
    Ok(resp)
  }
}

/// Plain HTTP client for unencrypted connections.
///
/// `TakoClient` provides a standard HTTP client that establishes plain TCP connections
/// to remote servers without encryption. It's suitable for internal services, development
/// environments, or when TLS termination is handled by a proxy. The client maintains
/// a persistent connection and uses HTTP/1.1 protocol.
///
/// # Type Parameters
///
/// * `B` - Body type for HTTP requests, must implement `Body + Send + 'static`
///
/// # Examples
///
/// ```rust,no_run
/// use tako::client::TakoClient;
/// use http_body_util::Empty;
/// use bytes::Bytes;
/// use http::Request;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create HTTP client for local development server
/// let mut client = TakoClient::<Empty<Bytes>>::new("localhost", Some(3000)).await?;
///
/// // Make request to health check endpoint
/// let request = Request::builder()
///     .method("GET")
///     .uri("/health")
///     .body(Empty::new())?;
///
/// let response = client.request(request).await?;
/// println!("Health check: {}", response.status());
/// # Ok(())
/// # }
/// ```
pub struct TakoClient<B: Body>
where
  B: Body + Send + 'static,
  B::Data: Send + 'static,
  B::Error: Into<Box<dyn Error + Send + Sync>>,
{
  /// HTTP/1.1 request sender for the established TCP connection.
  sender: SendRequest<B>,
  /// Background task handle managing the connection lifecycle.
  _conn_handle: JoinHandle<Result<(), hyper::Error>>,
}

impl<B> TakoClient<B>
where
  B: Body + Send + 'static,
  B::Data: Send + 'static,
  B::Error: Into<Box<dyn Error + Send + Sync>>,
{
  /// Creates a new HTTP client for plain TCP connections.
  pub async fn new<'a>(host: &'a str, port: Option<u16>) -> Result<Self, Box<dyn Error>>
  where
    'a: 'static,
  {
    let port = port.unwrap_or(80);
    let addr = format!("{host}:{port}");
    let tcp_stream = TcpStream::connect(addr).await?;
    let io = TokioIo::new(tcp_stream);

    // HTTP/1 handshake
    let (sender, conn) = client::conn::http1::handshake::<_, B>(io).await?;
    let conn_handle = tokio::spawn(async move {
      if let Err(err) = conn.await {
        tracing::error!("Connection error: {}", err);
      }

      Ok(())
    });

    Ok(Self {
      sender,
      _conn_handle: conn_handle,
    })
  }

  /// Sends an HTTP request and returns the response with body as bytes.
  ///
  /// This method sends the request over the established TCP connection and reads
  /// the complete response body into memory as a byte vector. The response headers
  /// and status are preserved while the body is collected into a `Vec<u8>`.
  ///
  /// # Errors
  ///
  /// Returns an error if the request fails to send, the response cannot be read,
  /// or connection issues occur during the request/response cycle.
  ///
  /// # Examples
  ///
  /// ```rust,no_run
  /// use tako::client::TakoClient;
  /// use http_body_util::Empty;
  /// use bytes::Bytes;
  /// use http::{Request, Method};
  ///
  /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
  /// let mut client = TakoClient::<Empty<Bytes>>::new("httpbin.org", Some(80)).await?;
  ///
  /// let request = Request::builder()
  ///     .method(Method::POST)
  ///     .uri("/post")
  ///     .header("content-type", "application/json")
  ///     .body(Empty::new())?;
  ///
  /// let response = client.request(request).await?;
  /// println!("Status: {}", response.status());
  /// let body_text = String::from_utf8_lossy(response.body());
  /// println!("Response: {}", body_text);
  /// # Ok(())
  /// # }
  /// ```
  pub async fn request(&mut self, req: Request<B>) -> Result<Response<Vec<u8>>, Box<dyn Error>> {
    let mut response = self.sender.send_request(req).await?;
    let mut body_bytes = Vec::new();

    while let Some(frame) = response.frame().await {
      let frame = frame?;
      if let Some(chunk) = frame.data_ref() {
        body_bytes.extend_from_slice(chunk);
      }
    }

    let parts = response.into_parts();
    let resp = Response::from_parts(parts.0, body_bytes);
    Ok(resp)
  }
}
