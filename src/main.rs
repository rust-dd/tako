use hyper::{Method, Request, body::Incoming};
use tako::responder::Responder;

pub async fn hello(_req: Request<Incoming>) -> impl Responder {
    "Hello, World!".into_response()
}

pub async fn user_created(_req: Request<Incoming>) -> impl Responder {
    String::from("User created").into_response()
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::new();
    r.route(Method::GET, "/", hello);
    r.route(Method::POST, "/user", user_created);
    tako::serve(listener, r).await;
}
