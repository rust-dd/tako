use hyper::{Request, server::conn::http1, service::service_fn};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::router::Router;
use crate::types::BoxError;

pub async fn run(listener: TcpListener, router: Router) -> Result<(), BoxError> {
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
