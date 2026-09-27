use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::router::Router;
use tako::types::Request;
use tako::types::Response;

#[path = "shared_stores/failures.rs"]
mod failures;
#[cfg(feature = "plugins")]
#[path = "shared_stores/idempotency.rs"]
mod idempotency;
#[path = "shared_stores/session_csrf.rs"]
mod session_csrf;

fn request(method: Method, path: &str, cookies: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .header("cookie", cookies)
    .body(TakoBody::empty())
    .unwrap()
}

fn cookies(response: &Response) -> String {
  response
    .headers()
    .get_all("set-cookie")
    .iter()
    .map(|value| value.to_str().unwrap().split(';').next().unwrap())
    .collect::<Vec<_>>()
    .join("; ")
}

async fn body(response: Response) -> String {
  String::from_utf8(
    response
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .to_vec(),
  )
  .unwrap()
}
