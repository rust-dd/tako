use hyper::{Method, Request, body::Incoming};
use tako::{extractors::state::State, responder::Responder, types::AppState as AppStateTrait};

#[derive(Clone, Default)]
struct AppState {
    pub count: u32,
}

impl AppStateTrait for AppState {}

pub async fn hello(_req: Request<Incoming>, State(state): State<AppState>) -> impl Responder {
    "Hello, World!".into_response()
}

pub async fn user_created(
    _req: Request<Incoming>,
    State(state): State<AppState>,
) -> impl Responder {
    String::from("User created").into_response()
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::<AppState>::new();
    let state = AppState { count: 0 };
    r.state(state);

    r.route(Method::GET, "/", hello);
    r.route(Method::POST, "/user", user_created);
    tako::serve(listener, r).await;
}
