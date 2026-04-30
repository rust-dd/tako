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

use http::Request;
use http::Response;
use http_body::Body;
use http_body_util::BodyExt;
use hyper::client::conn::http1::SendRequest;
use hyper::client::{self};
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use webpki_roots::TLS_SERVER_ROOTS;

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
