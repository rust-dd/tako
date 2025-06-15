use bytes::Bytes;
use http_body_util::Empty;
use hyper::{Request, body::Body, server::conn::http1, service::service_fn};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::router::Router;

pub async fn run<B>(
    listener: TcpListener,
    router: Router<B>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let router = Arc::new(router);
    println!("Tako listening on {}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);
        let io = hyper_util::rt::TokioIo::new(stream);
        let router = router.clone();

        tokio::spawn(async move {
            let svc = service_fn(|req: Request<_>| {
                let router = router.clone();
                async move { Ok::<_, Infallible>(router.dispatch(req).await) }
            });

            let http = http1::Builder::new();
            let conn = http.serve_connection(io, svc);

            if let Err(err) = conn.await {
                eprintln!("Error serving connection: {}", err);
            }
        });
    }
}
