use anyhow::Result;
use tako::{
    Method, plugins::compression::CompressionBuilder, responder::Responder, router::Router,
    types::Request,
};
use tokio::net::TcpListener;

use crate::text::TEXT;

mod text;

pub async fn compression(mut _req: Request) -> impl Responder {
    TEXT.into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/", compression);

    router.plugin(
        CompressionBuilder::new()
            .enable_gzip(true)
            .enable_brotli(true)
            .enable_stream(true)
            //.enable_zstd(true)
            .min_size(1024)
            .brotli_level(9)
            .build(),
    );

    tako::serve(listener, router).await;

    Ok(())
}
