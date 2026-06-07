//! HTTPS client with TLS encryption support using rustls.

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

use super::trust_store::load_root_certs;

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
  conn_handle: JoinHandle<Result<(), hyper::Error>>,
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
    load_root_certs(&mut root_cert_store);
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
      conn_handle,
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

impl<B> Drop for TakoTlsClient<B>
where
  B: Body + Send + 'static,
  B::Data: Send + 'static,
  B::Error: Into<Box<dyn Error + Send + Sync>>,
{
  fn drop(&mut self) {
    // Without this, dropping `conn_handle` simply detaches the task and
    // the background connection driver keeps running until the remote
    // closes (or forever for long-lived idle connections). Abort it so
    // the underlying TLS stream is dropped and any pending Tokio task
    // is cleared from the runtime.
    self.conn_handle.abort();
  }
}
