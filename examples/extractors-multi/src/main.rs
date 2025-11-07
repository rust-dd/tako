use anyhow::Result;
use serde::{Deserialize, Serialize};
use tako::extractors::{json::Json, params::Params, query::Query};
use tako::{Method, router::Router};
use tokio::net::TcpListener;

#[derive(Deserialize)]
struct Pagination {
    page: u32,
    per_page: u32,
}

#[derive(Deserialize)]
struct UserPath {
    id: u64,
}

#[derive(Deserialize, Serialize, Clone)]
struct CreateUser {
    name: String,
    email: String,
}

#[derive(Serialize)]
struct Created {
    id: u64,
    name: String,
    email: String,
}

// GET /users/{id}/posts?per_page=10&page=2
// Demonstrates multiple extractors: Params + Query
async fn list_user_posts(Params(user): Params<UserPath>, Query(p): Query<Pagination>) -> String {
    format!(
        "user_id={}, page={}, per_page={}",
        user.id, p.page, p.per_page
    )
}

// POST /users with JSON body {"name":"...","email":"..."}
// Demonstrates a body extractor
async fn create(Json(user): Json<CreateUser>) -> Json<Created> {
    // Normally you'd persist the user; here we just echo back with an id
    Json(Created {
        id: 1,
        name: user.name,
        email: user.email,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/users/{id}/posts", list_user_posts);
    router.route(Method::POST, "/users", create);

    tako::serve(listener, router).await;

    Ok(())
}
