use crate::{
    body::TakoBody,
    responder::Responder,
    types::{Request, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::FutureExt; // for catch_unwind
use http::header;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo; // <‑‑ adapter
use sha1::{Digest, Sha1};
use std::future::Future;
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

pub struct WsResponder<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    request: Request,
    handler: H,
}

impl<H, Fut> WsResponder<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    pub fn new(request: Request, handler: H) -> Self {
        Self { request, handler }
    }
}

impl<H, Fut> Responder for WsResponder<H, Fut>
where
    H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn into_response(self) -> Response {
        let (parts, body) = self.request.into_parts();
        let req = http::Request::from_parts(parts, body);

        let key = match req.headers().get("Sec-WebSocket-Key") {
            Some(k) => k,
            None => {
                return hyper::Response::builder()
                    .status(400)
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
            .status(101)
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
