use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::router::Router;
use tako::types::Request;

fn make_req(method: Method, uri: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::empty())
    .unwrap()
}

fn make_req_with_body(method: Method, uri: &str, body: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::from(body.to_string()))
    .unwrap()
}

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[path = "middleware/auth.rs"]
mod auth;
#[path = "middleware/chain.rs"]
mod chain;
#[path = "middleware/compression.rs"]
mod compression;
#[path = "middleware/csrf.rs"]
mod csrf;
#[path = "middleware/health.rs"]
mod health;
#[path = "middleware/response.rs"]
mod response;
#[path = "middleware/session.rs"]
mod session;
