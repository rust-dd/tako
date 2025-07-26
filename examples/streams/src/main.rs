use std::convert::Infallible;

use anyhow::Result;
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use hyper::body::Frame;
use hyper::{StatusCode, header};
use tako::{
    Method, body::TakoBody, bytes::TakoBytes, responder::Responder, router::Router, sse::Sse,
    types::Request,
};
use tokio::net::TcpListener;

async fn numbers(_: Request) -> impl Responder {
    let s = stream::iter(0u8..=9)
        .then(|n| async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Bytes::from(format!("{}\n", n))
        })
        .map(|b| Ok::<_, Infallible>(b));

    hyper::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(TakoBody::from_stream(s))
        .unwrap()
        .into_response()
}

async fn json_ticks(_: Request) -> impl Responder {
    let s = stream::unfold(0u64, |i| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if i % 5 == 4 {
            Some((
                Err::<Frame<Bytes>, _>(std::io::Error::new(std::io::ErrorKind::Other, "boom")),
                i + 1,
            ))
        } else {
            let payload = format!("{{\"tick\":{}}}", i);
            Some((Ok(Frame::data(Bytes::from(payload))), i + 1))
        }
    });

    hyper::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(TakoBody::from_try_stream(s))
        .unwrap()
        .into_response()
}

async fn ticker(_: Request) -> impl Responder {
    let s = stream::unfold(0u64, |i| async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let msg = format!("tick: {i}");
        Some((TakoBytes(msg.into()), i + 1))
    });

    Sse::new(s)
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/", numbers);
    router.route(Method::GET, "/json", json_ticks);
    router.route(Method::GET, "/events", ticker);

    tako::serve(listener, router).await;
    Ok(())
}
