use hyper::Method;
use tako::{
    extractors::{FromRequest, bytes::Bytes, header_map::HeaderMap},
    responder::Responder,
    state::get_state,
    types::Request,
};

#[derive(Clone, Default)]
pub struct AppState {
    pub count: u32,
}

pub async fn hello(mut req: Request) -> impl Responder {
    let HeaderMap(headers) = HeaderMap::from_request(&mut req).await.unwrap();
    let Bytes(bytes) = Bytes::from_request(&mut req).await.unwrap();

    "Hello, World!".into_response()
}

pub async fn user_created(_: Request) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    String::from("User created").into_response()
}

pub async fn middleware(req: Request) -> Request {
    // Your middleware logic here
    req
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
    r.route(Method::POST, "/user", user_created);
    tako::serve(listener, r).await;
}
