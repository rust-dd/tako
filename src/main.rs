use std::time::Duration;

use futures_util::StreamExt;
use hyper::Method;
use serde::Deserialize;
use tako::{middleware::Next, responder::Responder, sse::Sse, types::Request};
use tokio_stream::wrappers::IntervalStream;

#[cfg(feature = "plugins")]
use tako::plugins::{
    compression::CompressionBuilder, cors::CorsPlugin, rate_limiter::RateLimiterBuilder,
};

#[derive(Clone, Default)]
pub struct AppState {
    pub count: u32,
}

#[derive(Deserialize, Debug)]
pub struct UserCompanyParams {
    pub user_id: u32,
    pub company_id: u32,
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

pub async fn middleware1(req: Request, next: Next) -> impl Responder {
    println!("Middleware 1 executed");
    next.run(req).await.into_response()
}

pub async fn middleware2(req: Request, next: Next) -> impl Responder {
    println!("Middleware 2 executed");
    next.run(req).await.into_response()
}

pub async fn middleware3(req: Request, next: Next) -> impl Responder {
    println!("Middleware 3 executed");
    next.run(req).await.into_response()
}

pub async fn middleware4(req: Request, next: Next) -> impl Responder {
    println!("Middleware 4 executed");
    next.run(req).await.into_response()
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::new();

    r.route_with_tsr(Method::GET, "/sse/string", sse_string_handler);
    r.route_with_tsr(Method::GET, "/sse/bytes", sse_bytes_handler);

    r.middleware(middleware3).middleware(middleware4);

    #[cfg(feature = "plugins")]
    r.plugin(CorsPlugin::default());

    #[cfg(feature = "plugins")]
    r.plugin(
        RateLimiterBuilder::new()
            .burst_size(5)
            .per_second(20)
            .tick_secs(20)
            .build(),
    );

    #[cfg(feature = "plugins")]
    r.plugin(
        CompressionBuilder::new()
            .enable_gzip(true)
            .enable_brotli(true)
            //.enable_zstd(true)
            .min_size(1024)
            .build(),
    );

    #[cfg(not(feature = "tls"))]
    tako::serve(listener, r).await;

    #[cfg(feature = "tls")]
    tako::serve_tls(listener, r, None, None).await;
}
