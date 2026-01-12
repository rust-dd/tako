//! OpenAPI documentation example for Tako.
//!
//! This example demonstrates how to use route-level OpenAPI metadata
//! to generate an OpenAPI specification from your Tako routes.
//!
//! Run with: cargo run --example openapi --features vespera

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tako::body::TakoBody;
use tako::extractors::json::Json;
use tako::extractors::params::Params;
use tako::extractors::query::Query;
use tako::openapi::ui::{Scalar, SwaggerUi};
use tako::openapi::vespera::{generate_openapi_from_routes, Info, VesperaOpenApiJson};
use tako::openapi::{OpenApiParameter, OpenApiRequestBody, ParameterLocation};
use tako::responder::Responder;
use tako::router::Router;
use tako::types::{Request, Response};
use tako::Method;
use tokio::net::TcpListener;

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

#[derive(Debug, Deserialize)]
struct IdParam {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Serialize)]
struct UserList {
    users: Vec<User>,
    total: u64,
}

#[derive(Debug, Deserialize)]
struct CreateUser {
    name: String,
    email: String,
}

async fn list_users(Query(query): Query<ListQuery>) -> impl Responder {
    let limit = query.limit.unwrap_or(10);
    let offset = query.offset.unwrap_or(0);

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
        users: users.into_iter().skip(offset as usize).take(limit as usize).collect(),
        total: 2,
    })
}

async fn get_user(Params(params): Params<IdParam>) -> Response {
    if params.id == 1 {
        Json(User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        })
        .into_response()
    } else {
        http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .body(TakoBody::from("User not found"))
            .unwrap()
    }
}

async fn create_user(Json(data): Json<CreateUser>) -> Response {
    let user = User {
        id: 3,
        name: data.name,
        email: data.email,
    };

    let mut res = Json(user).into_response();
    *res.status_mut() = http::StatusCode::CREATED;
    res
}

async fn delete_user(Params(params): Params<IdParam>) -> Response {
    if params.id == 1 {
        http::Response::builder()
            .status(http::StatusCode::NO_CONTENT)
            .body(TakoBody::empty())
            .unwrap()
    } else {
        http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .body(TakoBody::from("User not found"))
            .unwrap()
    }
}

async fn health() -> impl Responder {
    Json(serde_json::json!({ "status": "healthy" }))
}

fn setup_routes(router: &mut Router) {
    // Health check endpoint
    router
        .route(Method::GET, "/health", health)
        .operation_id("healthCheck")
        .summary("Health check")
        .description("Returns the health status of the API")
        .tag("system")
        .response(200, "API is healthy");

    // List users
    router
        .route(Method::GET, "/users", list_users)
        .operation_id("listUsers")
        .summary("List all users")
        .description("Retrieves a paginated list of all users in the system")
        .tag("users")
        .parameter(OpenApiParameter {
            name: "limit".to_string(),
            location: ParameterLocation::Query,
            description: Some("Maximum number of users to return (default: 10)".to_string()),
            required: false,
        })
        .parameter(OpenApiParameter {
            name: "offset".to_string(),
            location: ParameterLocation::Query,
            description: Some("Number of users to skip (default: 0)".to_string()),
            required: false,
        })
        .response(200, "List of users");

    // Get user by ID
    router
        .route(Method::GET, "/users/{id}", get_user)
        .operation_id("getUser")
        .summary("Get user by ID")
        .description("Retrieves a specific user by their unique identifier")
        .tag("users")
        .parameter(OpenApiParameter {
            name: "id".to_string(),
            location: ParameterLocation::Path,
            description: Some("Unique user identifier".to_string()),
            required: true,
        })
        .response(200, "User found")
        .response(404, "User not found");

    // Create user
    router
        .route(Method::POST, "/users", create_user)
        .operation_id("createUser")
        .summary("Create a new user")
        .description("Creates a new user with the provided data")
        .tag("users")
        .request_body(OpenApiRequestBody {
            description: Some("User data".to_string()),
            required: true,
            content_type: "application/json".to_string(),
        })
        .response(201, "User created successfully")
        .response(400, "Invalid request body");

    // Delete user
    router
        .route(Method::DELETE, "/users/{id}", delete_user)
        .operation_id("deleteUser")
        .summary("Delete a user")
        .description("Permanently removes a user from the system")
        .tag("users")
        .parameter(OpenApiParameter {
            name: "id".to_string(),
            location: ParameterLocation::Path,
            description: Some("User ID to delete".to_string()),
            required: true,
        })
        .response(204, "User deleted successfully")
        .response(404, "User not found")
        .security("bearerAuth");
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();

    // Setup API routes with OpenAPI metadata
    setup_routes(&mut router);

    // Create OpenAPI spec info
    let info = Info {
        title: "Tako Example API".to_string(),
        version: "1.0.0".to_string(),
        description: Some("A sample API demonstrating Tako's OpenAPI integration".to_string()),
        terms_of_service: None,
        contact: None,
        license: None,
        summary: None,
    };

    // Generate OpenAPI spec from routes
    let spec = generate_openapi_from_routes(&router, info);

    // Serve OpenAPI JSON at /openapi.json
    router.route(Method::GET, "/openapi.json", move |_: Request| {
        let spec = spec.clone();
        async move { VesperaOpenApiJson(spec) }
    });

    // Serve Swagger UI at /docs
    router.route(Method::GET, "/docs", |_: Request| async {
        SwaggerUi::new("/openapi.json").title("Tako API - Swagger UI")
    });

    // Serve Scalar UI at /scalar
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
