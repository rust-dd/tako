#![cfg(feature = "vespera")]

use http::Method;
use tako_rs_core::openapi::vespera::HttpMethod;
use tako_rs_core::openapi::vespera::Info;
use tako_rs_core::openapi::vespera::generate_openapi_from_routes;
use tako_rs_core::openapi::vespera::http_method_to_vespera;
use tako_rs_core::router::Router;

#[test]
fn unsupported_method_mapping_is_explicit() {
  assert!(matches!(
    http_method_to_vespera(&Method::GET),
    Some(HttpMethod::Get)
  ));
  assert!(http_method_to_vespera(&Method::CONNECT).is_none());
  assert!(http_method_to_vespera(&Method::from_bytes(b"CUSTOM").unwrap()).is_none());
}

#[test]
fn unsupported_methods_never_create_or_replace_get_operations() {
  let mut router = Router::new();
  router
    .get("/shared", || async { "get" })
    .summary("Real GET");
  for method in [Method::CONNECT, Method::from_bytes(b"CUSTOM").unwrap()] {
    router
      .route(method.clone(), "/shared", || async { "other" })
      .summary("Unsupported");
    router
      .route(method, "/unsupported", || async { "other" })
      .summary("Unsupported");
  }

  let spec = generate_openapi_from_routes(&router, Info::default());
  let json = serde_json::to_value(spec).unwrap();
  assert_eq!(json["paths"]["/shared"]["get"]["summary"], "Real GET");
  assert_eq!(json["paths"]["/shared"].as_object().unwrap().len(), 1);
  assert!(json["paths"].get("/unsupported").is_none());
}

#[test]
fn supported_methods_keep_their_own_operations() {
  let mut router = Router::new();
  for method in [
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
    Method::HEAD,
    Method::OPTIONS,
    Method::TRACE,
  ] {
    let summary = method.as_str().to_owned();
    router
      .route(method, "/supported", || async { "ok" })
      .summary(summary);
  }
  let spec = generate_openapi_from_routes(&router, Info::default());
  let json = serde_json::to_value(spec).unwrap();
  for method in [
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
  ] {
    assert_eq!(
      json["paths"]["/supported"][method]["summary"],
      method.to_uppercase()
    );
  }
}
