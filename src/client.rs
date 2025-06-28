use std::{error::Error, sync::Arc};

use http_body_util::BodyExt;
use hyper::{
    Request, Response,
    body::Body,
    client::{self, conn::http1::SendRequest},
};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::{net::TcpStream, task::JoinHandle};
use tokio_rustls::TlsConnector;
use webpki_roots::TLS_SERVER_ROOTS;

pub struct TakoTlsClient<B: Body>
where
    B: Body + Send + 'static,
    B::Data: Send + 'static,
    B::Error: Into<Box<dyn Error + Send + Sync>>,
{
    sender: SendRequest<B>,
    _conn_handle: JoinHandle<Result<(), hyper::Error>>,
}

impl<B> TakoTlsClient<B>
where
    B: Body + Send + 'static,
    B::Data: Send + 'static,
    B::Error: Into<Box<dyn Error + Send + Sync>>,
{
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

pub struct TakoClient<B: Body>
where
    B: Body + Send + 'static,
    B::Data: Send + 'static,
    B::Error: Into<Box<dyn Error + Send + Sync>>,
{
    sender: SendRequest<B>,
    _conn_handle: JoinHandle<Result<(), hyper::Error>>,
}

impl<B> TakoClient<B>
where
    B: Body + Send + 'static,
    B::Data: Send + 'static,
    B::Error: Into<Box<dyn Error + Send + Sync>>,
{
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
