use anyhow::Result;
use tako::{
    Method,
    middleware::{IntoMiddleware, basic_auth, bearer_auth},
    responder::Responder,
    types::Request,
};

async fn basic_auth_route(_: Request) -> impl Responder {
    "Basic Auth Route"
}

async fn bearer_auth_route(_: Request) -> impl Responder {
    "Bearer Auth Route"
}

async fn basic_auth_with_verify(_: Request) -> impl Responder {
    "Basic Auth Route with Verify"
}

async fn bearer_auth_with_verify(_: Request) -> impl Responder {
    "Bearer Auth Route with Verify"
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut router = tako::router::Router::new();

    let basic = basic_auth::BasicAuth::single("admin", "pw")
        .realm("Admin Area")
        .into_middleware();
    let bearer = bearer_auth::BearerAuth::static_token("my-secret-token").into_middleware();

    router
        .route_with_tsr(Method::GET, "/basic", basic_auth_route)
        .middleware(basic);
    router
        .route_with_tsr(Method::POST, "/bearer", bearer_auth_route)
        .middleware(bearer);

    let basic_with_verify =
        basic_auth::BasicAuth::with_verify(|user, password| user == "admin" && password == "pw")
            .realm("Admin Area")
            .into_middleware();
    let bearer_with_verify =
        bearer_auth::BearerAuth::with_verify(|token| token == "my-secret-token").into_middleware();

    router
        .route_with_tsr(Method::GET, "/basic_with_verify", basic_auth_with_verify)
        .middleware(basic_with_verify);
    router
        .route_with_tsr(Method::POST, "/bearer_with_verify", bearer_auth_with_verify)
        .middleware(bearer_with_verify);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
