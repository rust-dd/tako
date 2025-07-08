/// This module provides the `TakoWs` struct, which is used to handle WebSocket connections.
///
/// The `TakoWs` struct allows for the creation of WebSocket handlers that can process
/// WebSocket streams and respond to WebSocket upgrade requests.
use crate::{
    body::TakoBody,
    responder::Responder,
    types::{Request, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::FutureExt;
use http::{StatusCode, header};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use std::future::Future;
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

/// The `TakoWs` struct represents a WebSocket handler.
///
/// This struct is used to handle WebSocket upgrade requests and process WebSocket streams.
///
/// # Example
///
/// ```rust
/// use tako::ws::TakoWs;
/// use hyper::Request;
/// use tokio_tungstenite::WebSocketStream;
/// use hyper_util::rt::TokioIo;
/// use futures_util::StreamExt;
///
/// async fn websocket_handler(ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>) {
///     let (mut write, mut read) = ws.split();
///     while let Some(Ok(msg)) = read.next().await {
///         // Process WebSocket messages here
///     }
/// }
///
/// let request: Request<_> = /* incoming request */;
/// let ws_handler = TakoWs::new(request, websocket_handler);
/// ```
///
/// # Type Parameters
///
/// * `H` - The handler function type.
/// * `Fut` - The future returned by the handler function.
pub struct TakoWs<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    request: Request,
    handler: H,
}

impl<H, Fut> TakoWs<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    /// Creates a new `TakoWs` instance.
    ///
    /// # Arguments
    ///
    /// * `request` - The incoming HTTP request to be upgraded to a WebSocket connection.
    /// * `handler` - A function that processes the WebSocket stream.
    ///
    /// # Returns
    ///
    /// A new instance of `TakoWs`.
    pub fn new(request: Request, handler: H) -> Self {
        Self { request, handler }
    }
}

impl<H, Fut> Responder for TakoWs<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    /// Converts the `TakoWs` instance into an HTTP response.
    ///
    /// This method handles the WebSocket upgrade request and spawns a task to process
    /// the WebSocket stream using the provided handler.
    ///
    /// # Returns
    ///
    /// An HTTP response indicating the result of the WebSocket upgrade.
    fn into_response(self) -> Response {
        let (parts, body) = self.request.into_parts();
        let req = http::Request::from_parts(parts, body);

        let key = match req.headers().get("Sec-WebSocket-Key") {
            Some(k) => k,
            None => {
                return hyper::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(TakoBody::from("Missing Sec-WebSocket-Key".to_string()))
                    .unwrap();
            }
        };

        // RFC‑6455 accept hash
        let accept = {
            let mut sha1 = Sha1::new();
            sha1.update(key.as_bytes());
            sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            STANDARD.encode(sha1.finalize())
        };

        let response = hyper::Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(header::UPGRADE, "websocket")
            .header(header::CONNECTION, "Upgrade")
            .header("Sec-WebSocket-Accept", accept)
            .body(TakoBody::empty())
            .unwrap();

        if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
            let handler = self.handler;
            tokio::spawn(async move {
                if let Ok(upgraded) = on_upgrade.await {
                    let upgraded = TokioIo::new(upgraded);
                    let ws = WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await;
                    let _ = std::panic::AssertUnwindSafe(handler(ws))
                        .catch_unwind()
                        .await;
                }
            });
        }

        response
    }
}
