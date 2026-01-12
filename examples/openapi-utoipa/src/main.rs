//! OpenAPI documentation example using utoipa for Tako.
//!
//! This example demonstrates how to use utoipa's derive macros to generate
//! OpenAPI documentation with Tako's route-level metadata support.
//!
//! Run with: cargo run --example openapi-utoipa --features utoipa

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tako::body::TakoBody;
use tako::extractors::json::Json;
use tako::extractors::params::Params;
use tako::extractors::query::Query;
use tako::openapi::ui::{Scalar, SwaggerUi};
use tako::openapi::utoipa::{OpenApi, OpenApiJson, ToSchema};
use tako::responder::Responder;
use tako::router::Router;
use tako::types::{Request, Response};
use tako::Method;
use tokio::net::TcpListener;

/// User model for the API.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct User {
    /// Unique user identifier.
    id: u64,
    /// User's display name.
    name: String,
    /// User's email address.
    email: String,
}

/// Path parameters for user operations.
#[derive(Debug, Deserialize)]
struct IdParam {
    id: u64,
}

/// Query parameters for listing users.
#[derive(Debug, Deserialize, ToSchema)]
struct ListQuery {
    /// Maximum number of users to return.
    #[serde(default = "default_limit")]
    limit: u32,
    /// Number of users to skip.
    #[serde(default)]
    offset: u32,
}

fn default_limit() -> u32 {
    10
}

/// Response for listing users.
#[derive(Debug, Serialize, ToSchema)]
struct UserList {
    /// List of users.
    users: Vec<User>,
    /// Total number of users.
    total: u64,
}

/// Request body for creating a user.
#[derive(Debug, Deserialize, ToSchema)]
struct CreateUserRequest {
    /// User's display name.
    name: String,
    /// User's email address.
    email: String,
}

/// Health check response.
#[derive(Debug, Serialize, ToSchema)]
struct HealthResponse {
    /// Health status.
    status: String,
}

/// Error response.
#[derive(Debug, Serialize, ToSchema)]
struct ErrorResponse {
    /// Error message.
    message: String,
}

/// List all users with pagination.
#[utoipa::path(
    get,
    path = "/users",
    params(
        ("limit" = Option<u32>, Query, description = "Maximum number of users to return"),
        ("offset" = Option<u32>, Query, description = "Number of users to skip")
    ),
    responses(
        (status = 200, description = "List of users", body = UserList)
    ),
    tag = "users"
)]
async fn list_users(Query(query): Query<ListQuery>) -> impl Responder {
    let users = vec![
        User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        },
        User {
            id: 2,
            name: "Bob".to_string(),
            email: "bob@example.com".to_string(),
        },
    ];

    Json(UserList {
        users: users
            .into_iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .collect(),
        total: 2,
    })
}

/// Get a user by ID.
#[utoipa::path(
    get,
    path = "/users/{id}",
    params(
        ("id" = u64, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "User found", body = User),
        (status = 404, description = "User not found", body = ErrorResponse)
    ),
    tag = "users"
)]
async fn get_user(Params(params): Params<IdParam>) -> Response {
    if params.id == 1 {
        Json(User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        })
        .into_response()
    } else {
        let mut res = Json(ErrorResponse {
            message: "User not found".to_string(),
        })
        .into_response();
        *res.status_mut() = http::StatusCode::NOT_FOUND;
        res
    }
}

/// Create a new user.
#[utoipa::path(
    post,
    path = "/users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = User),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    ),
    tag = "users"
)]
async fn create_user(Json(data): Json<CreateUserRequest>) -> Response {
    let user = User {
        id: 3,
        name: data.name,
        email: data.email,
    };

    let mut res = Json(user).into_response();
    *res.status_mut() = http::StatusCode::CREATED;
    res
}

/// Delete a user by ID.
#[utoipa::path(
    delete,
    path = "/users/{id}",
    params(
        ("id" = u64, Path, description = "User ID to delete")
    ),
    responses(
        (status = 204, description = "User deleted"),
        (status = 404, description = "User not found", body = ErrorResponse)
    ),
    tag = "users",
    security(
        ("bearer_auth" = [])
    )
)]
async fn delete_user(Params(params): Params<IdParam>) -> Response {
    if params.id == 1 {
        http::Response::builder()
            .status(http::StatusCode::NO_CONTENT)
            .body(TakoBody::empty())
            .unwrap()
    } else {
        let mut res = Json(ErrorResponse {
            message: "User not found".to_string(),
        })
        .into_response();
        *res.status_mut() = http::StatusCode::NOT_FOUND;
        res
    }
}

/// Health check endpoint.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "API is healthy", body = HealthResponse)
    ),
    tag = "system"
)]
async fn health() -> impl Responder {
    Json(HealthResponse {
        status: "healthy".to_string(),
    })
}

/// OpenAPI documentation for the API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Tako Example API",
        version = "1.0.0",
        description = "A sample API demonstrating Tako's utoipa integration"
    ),
    paths(
        list_users,
        get_user,
        create_user,
        delete_user,
        health
    ),
    components(
        schemas(User, UserList, CreateUserRequest, HealthResponse, ErrorResponse, ListQuery)
    ),
    tags(
        (name = "users", description = "User management operations"),
        (name = "system", description = "System operations")
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();

    // API routes
    router.route(Method::GET, "/health", health);
    router.route(Method::GET, "/users", list_users);
    router.route(Method::GET, "/users/{id}", get_user);
    router.route(Method::POST, "/users", create_user);
    router.route(Method::DELETE, "/users/{id}", delete_user);

    // OpenAPI spec endpoint
    router.route(Method::GET, "/openapi.json", |_: Request| async {
        OpenApiJson(ApiDoc::openapi())
    });

    // Swagger UI
    router.route(Method::GET, "/docs", |_: Request| async {
        SwaggerUi::new("/openapi.json").title("Tako API - Swagger UI")
    });

    // Scalar UI
    router.route(Method::GET, "/scalar", |_: Request| async {
        Scalar::new("/openapi.json").title("Tako API - Scalar")
    });

    println!("Server running at http://127.0.0.1:8080");
    println!("OpenAPI spec: http://127.0.0.1:8080/openapi.json");
    println!("Swagger UI:   http://127.0.0.1:8080/docs");
    println!("Scalar UI:    http://127.0.0.1:8080/scalar");

    tako::serve(listener, router).await;

    Ok(())
}
