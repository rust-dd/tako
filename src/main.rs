use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hyper::Method;
use serde::Deserialize;
use tako::{
    body::TakoBody,
    extractors::{FromRequest, bytes::Bytes, header_map::HeaderMap, params::Params},
    middleware::Next,
    responder::Responder,
    sse::Sse,
    state::get_state,
    types::{Request, Response},
    ws::TakoWs,
};
use tokio_stream::wrappers::IntervalStream;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

#[derive(Clone, Default)]
pub struct AppState {
    pub count: u32,
}

pub async fn hello(mut req: Request) -> impl Responder {
    let HeaderMap(headers) = HeaderMap::from_request(&mut req).await.unwrap();
    let Bytes(bytes) = Bytes::from_request(&mut req).await.unwrap();

    "Hello, World!".into_response()
}

#[derive(Deserialize, Debug)]
pub struct Par {
    pub id: u32,
}

pub async fn user_created(mut req: Request) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    let Params(params) = Params::<Par>::from_request(&mut req).await.unwrap();
    println!("User ID: {:?}", params);

    String::from("User created").into_response()
}

#[derive(Deserialize, Debug)]
pub struct UserCompanyParams {
    pub user_id: u32,
    pub company_id: u32,
}

pub async fn user_company(mut req: Request) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    let Params(params) = Params::<UserCompanyParams>::from_request(&mut req)
        .await
        .unwrap();
    println!("User ID: {:?}", params);

    String::from("User created").into_response()
}

pub async fn sse_string_handler(_: Request) -> impl Responder {
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .map(|_| "Hello".to_string().into());

    Sse::new(stream)
}

pub async fn sse_bytes_handler(_: Request) -> impl Responder {
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .map(|_| bytes::Bytes::from("hello").into());

    Sse::new(stream)
}

pub async fn ws_echo(req: Request) -> impl Responder {
    TakoWs::new(req, |mut ws| async move {
        let _ = ws.send(Message::Text("Welcome to Tako WS!".into())).await;

        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Text(txt) => {
                    let _ = ws
                        .send(Message::Text(Utf8Bytes::from(format!("Echo: {txt}"))))
                        .await;
                }
                Message::Binary(bin) => {
                    let _ = ws.send(Message::Binary(bin)).await;
                }
                Message::Ping(p) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Message::Close(_) => {
                    let _ = ws.send(Message::Close(None)).await;
                    break;
                }
                _ => {}
            }
        }
    })
}

pub async fn ws_tick(req: Request) -> impl Responder {
    TakoWs::new(req, |mut ws| async move {
        let mut ticker =
            IntervalStream::new(tokio::time::interval(Duration::from_secs(1))).enumerate();

        loop {
            tokio::select! {
                msg = ws.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }

                Some((i, _)) = ticker.next() => {
                    let _ = ws.send(Message::Text(Utf8Bytes::from(format!("tick #{i}")))).await;
                }
            }
        }
    })
}

pub async fn middleware(req: Request, next: Next<'_>) -> impl Responder {
    next.run(req).await.into_response()
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::new();
    r.state("app_state", AppState::default());

    r.route(Method::GET, "/", hello)
        .middleware(middleware)
        .middleware(middleware);
    r.route_with_tsr(Method::POST, "/user", user_created);
    r.route_with_tsr(Method::POST, "/user/{id}", user_created);
    r.route_with_tsr(
        Method::POST,
        "/user/{user_id}/company/{company_id}",
        user_company,
    );
    r.route_with_tsr(Method::GET, "/sse/string", sse_string_handler);
    r.route_with_tsr(Method::GET, "/sse/bytes", sse_bytes_handler);
    r.route_with_tsr(Method::GET, "/ws/echo", ws_echo);
    r.route_with_tsr(Method::GET, "/ws/tick", ws_tick);

    r.middleware(middleware).middleware(middleware);

    #[cfg(not(feature = "tls"))]
    tako::serve(listener, r).await;

    #[cfg(feature = "tls")]
    tako::serve_tls(listener, r, None, None).await;
}
