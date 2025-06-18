use http::request::Parts;
use hyper::Method;
use tako::{
    handler::{FromRequest, FromRequestParts},
    responder::Responder,
    state::get_state,
    types::{AppState as AppStateTrait, Request as TakoRequest},
};

pub struct Test;
pub struct Test1;
pub struct Test2;

impl<S, M> FromRequest<S, M> for Test {
    type Rejection = ();

    fn from_request(
        _req: TakoRequest,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test) }
    }
}

impl<S> FromRequestParts<S> for Test1 {
    type Rejection = ();

    fn from_request_parts(
        _req: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test1) }
    }
}

impl<S> FromRequestParts<S> for Test2 {
    type Rejection = ();

    fn from_request_parts(
        _req: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async { Ok(Test2) }
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    pub count: u32,
}

pub async fn hello(_: TakoRequest) -> impl Responder {
    "Hello, World!".into_response()
}

pub async fn user_created(_: TakoRequest) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    String::from("User created").into_response()
}

pub async fn middleware(req: TakoRequest) -> TakoRequest {
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
