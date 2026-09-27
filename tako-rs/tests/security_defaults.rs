use std::time::Duration;

use http::Version;
use http::header;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::middleware::alt_svc::AltSvc;
use tako::middleware::csrf::Csrf;
use tako::middleware::session::SessionMiddleware;
use tako::router::Router;
use tako::types::Request;

fn run(future: impl std::future::Future<Output = ()>) {
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(future);
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new().unwrap().block_on(future);
}

#[test]
fn session_and_csrf_cookies_are_secure_unless_explicitly_disabled() {
  run(async {
    for secure in [true, false] {
      let mut router = Router::new();
      let session = if secure {
        SessionMiddleware::new()
      } else {
        SessionMiddleware::new().secure(false)
      };
      let csrf = if secure {
        Csrf::new()
      } else {
        Csrf::new().secure(false)
      };
      router.middleware(session.into_middleware());
      router.middleware(csrf.into_middleware());
      router.get("/", || async { "ok" });
      let response = router.dispatch(Request::new(TakoBody::empty())).await;
      let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
      assert_eq!(cookies.len(), 2);
      for cookie in cookies {
        assert_eq!(
          cookie.split(';').any(|part| part.trim() == "Secure"),
          secure,
          "{cookie}"
        );
      }
    }
  });
}

#[test]
fn alt_svc_advertises_h3_preserves_overrides_and_skips_h3_requests() {
  run(async {
    let mut router = Router::new();
    router.middleware(AltSvc::h3(8443, Duration::from_secs(3600)).into_middleware());
    router.get("/", || async { "ok" });
    router.get("/clear", || async {
      http::Response::builder()
        .header(header::ALT_SVC, "clear")
        .body(TakoBody::empty())
        .unwrap()
    });
    for version in [Version::HTTP_11, Version::HTTP_2, Version::HTTP_3] {
      let request = http::Request::builder()
        .version(version)
        .body(TakoBody::empty())
        .unwrap();
      let response = router.dispatch(request).await;
      if version == Version::HTTP_3 {
        assert!(!response.headers().contains_key(header::ALT_SVC));
      } else {
        assert_eq!(response.headers()[header::ALT_SVC], "h3=\":8443\"; ma=3600");
      }
    }
    let request = http::Request::builder()
      .uri("/clear")
      .body(TakoBody::empty())
      .unwrap();
    assert_eq!(
      router.dispatch(request).await.headers()[header::ALT_SVC],
      "clear"
    );
  });
}
