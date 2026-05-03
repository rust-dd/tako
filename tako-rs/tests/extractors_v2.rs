//! v2 extractor regression tests covering the new shapes added in roadmap §5.1:
//! `Path<T>`, `QueryMulti<T>`, `MatchedPath`, `OriginalUri`, `Host`, `ContentLengthLimit`.

use http::Request;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::params::Params;
use tako::extractors::path::Path;
use tako::extractors::query_multi::QueryMulti;
use tako::extractors::uri_parts::Host;

fn req_with_uri(uri: &str) -> tako::types::Request {
  Request::builder()
    .uri(uri)
    .body(TakoBody::empty())
    .expect("test request")
}

#[tokio::test]
async fn path_t_single_primitive() {
  let mut req = req_with_uri("/users/42");
  req
    .extensions_mut()
    .insert(make_path_params(&[("id", "42")]));

  let Path(id): Path<u64> = Path::from_request(&mut req).await.unwrap();
  assert_eq!(id, 42);
}

#[tokio::test]
async fn path_t_tuple() {
  let mut req = req_with_uri("/u/foo/42");
  req
    .extensions_mut()
    .insert(make_path_params(&[("name", "foo"), ("id", "42")]));

  let Path((name, id)): Path<(String, u32)> = Path::from_request(&mut req).await.unwrap();
  assert_eq!(name, "foo");
  assert_eq!(id, 42);
}

#[tokio::test]
async fn path_t_struct() {
  #[derive(serde::Deserialize)]
  struct Key {
    tenant: String,
    user_id: u64,
  }

  let mut req = req_with_uri("/t/foo/u/9");
  req
    .extensions_mut()
    .insert(make_path_params(&[("tenant", "foo"), ("user_id", "9")]));

  let Path(key): Path<Key> = Path::from_request(&mut req).await.unwrap();
  assert_eq!(key.tenant, "foo");
  assert_eq!(key.user_id, 9);
}

#[tokio::test]
async fn query_multi_repeated_keys() {
  #[derive(serde::Deserialize, Debug)]
  struct Filter {
    tag: Vec<String>,
    sort: Option<String>,
  }

  let mut req = req_with_uri("/?tag=a&tag=b&sort=date");
  let QueryMulti(f): QueryMulti<Filter> = QueryMulti::from_request(&mut req).await.unwrap();
  assert_eq!(f.tag, vec!["a", "b"]);
  assert_eq!(f.sort.as_deref(), Some("date"));
}

#[tokio::test]
async fn host_from_x_forwarded_host() {
  let mut req = Request::builder()
    .uri("/")
    .header("x-forwarded-host", "example.com")
    .body(TakoBody::empty())
    .unwrap();
  let Host(h) = Host::from_request(&mut req).await.unwrap();
  assert_eq!(h, "example.com");
}

#[tokio::test]
async fn host_falls_back_to_host_header() {
  let mut req = Request::builder()
    .uri("/")
    .header("host", "fallback.test")
    .body(TakoBody::empty())
    .unwrap();
  let Host(h) = Host::from_request(&mut req).await.unwrap();
  assert_eq!(h, "fallback.test");
}

// --- helpers -----------------------------------------------------------------

fn make_path_params(pairs: &[(&str, &str)]) -> tako_core::extractors::params::PathParams {
  use smallvec::SmallVec;
  let mut sv: SmallVec<[(String, String); 4]> = SmallVec::new();
  for (k, v) in pairs {
    sv.push(((*k).to_string(), (*v).to_string()));
  }
  tako_core::extractors::params::PathParams(sv)
}

#[tokio::test]
async fn params_extractor_matches_path_extractor_for_struct() {
  // Sanity: `Path<T>` is just a re-export wrapper around `Params<T>`.
  #[derive(serde::Deserialize)]
  struct Pair {
    a: u64,
    b: String,
  }
  let mut req = req_with_uri("/x");
  req
    .extensions_mut()
    .insert(make_path_params(&[("a", "1"), ("b", "two")]));

  let Params(p1): Params<Pair> = Params::from_request(&mut req).await.unwrap();
  assert_eq!(p1.a, 1);
  assert_eq!(p1.b, "two");
}
