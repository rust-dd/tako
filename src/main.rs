use bytes::Bytes;
use http_body_util::Empty;
use hyper::{Method, Request, Response, body::Incoming};

pub async fn hello(_req: Request<Incoming>) -> Response<Empty<Bytes>> {
    Response::new(Empty::new())
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::new();
    r.route(Method::GET, "/", hello);
    r.route(Method::POST, "/user", |_| async move {
        Response::new(Empty::new())
    });
    tako::serve(listener, r).await;
}
