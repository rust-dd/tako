#![cfg(feature = "vespera")]

use http::Method;
use tako::openapi::vespera::Info;
use tako::openapi::vespera::generate_openapi_from_routes;
use tako::router::Router;
use tako::types::Request;

#[test]
fn vespera_spec_marks_deprecated_routes() {
  let mut router = Router::new();
  router
    .route(Method::GET, "/legacy", |_req: Request| async { "legacy" })
    .deprecated();
  router
    .route(Method::GET, "/current", |_req: Request| async { "current" })
    .summary("Current endpoint");

  let spec = generate_openapi_from_routes(&router, Info::default());
  let json = serde_json::to_value(spec).unwrap();
  assert_eq!(json["paths"]["/legacy"]["get"]["deprecated"], true);
  assert_ne!(json["paths"]["/current"]["get"]["deprecated"], true);
}
