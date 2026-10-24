use std::sync::Arc;

use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::middleware::session::Session;
use tako::middleware::session::SessionMiddleware;
use tako::middleware::session::SessionStoreHandle;
use tako::router::Router;
use tako::stores::memory::MemorySessionStore;
use tako::types::Request;
use tako::types::Response;
use tokio::sync::Notify;

#[derive(Clone, Default)]
struct Pause {
  entered: Arc<Notify>,
  release: Arc<Notify>,
}

fn router(store: MemorySessionStore, pause: Pause) -> (Router, SessionStoreHandle) {
  let sessions = SessionMiddleware::new().store(store);
  let handle = sessions.handle();
  let mut router = Router::new();
  router.middleware(sessions.into_middleware());
  router.post("/login", |request: Request| async move {
    request
      .extensions()
      .get::<Session>()
      .unwrap()
      .set("user", "alice");
    "logged in"
  });
  router.get("/whoami", |request: Request| async move {
    request
      .extensions()
      .get::<Session>()
      .unwrap()
      .get::<String>("user")
      .unwrap_or_else(|| "anonymous".into())
  });
  router.post("/logout", |request: Request| async move {
    request.extensions().get::<Session>().unwrap().destroy();
    "logged out"
  });
  router.post("/rotate", |request: Request| async move {
    request.extensions().get::<Session>().unwrap().rotate();
    "rotated"
  });
  router.post("/slow/{action}", move |request: Request| {
    let pause = pause.clone();
    async move {
      let session = request.extensions().get::<Session>().unwrap();
      assert_eq!(session.get::<String>("user").as_deref(), Some("alice"));
      pause.entered.notify_one();
      pause.release.notified().await;
      session.set("visited", true);
      if request.uri().path().ends_with("rotate") {
        session.rotate();
      }
      "finished"
    }
  });
  (router, handle)
}

fn request(method: Method, path: &str, cookie: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .header("cookie", cookie)
    .body(TakoBody::empty())
    .unwrap()
}

fn cookie(response: &Response) -> String {
  response.headers()["set-cookie"]
    .to_str()
    .unwrap()
    .split(';')
    .next()
    .unwrap()
    .to_owned()
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

enum Revoke {
  Logout,
  Rotate,
  All,
  Matching,
}

async fn late_request_cannot_restore_session(revoke: Revoke, rotate_late: bool) {
  let store = MemorySessionStore::new();
  let pause = Pause::default();
  let (first, handle) = router(store.clone(), pause.clone());
  let (second, _) = router(store, Pause::default());
  let original = cookie(&first.dispatch(request(Method::POST, "/login", "")).await);
  let path = if rotate_late {
    "/slow/rotate"
  } else {
    "/slow/save"
  };
  let late = first.dispatch(request(Method::POST, path, &original));
  let invalidate = async {
    pause.entered.notified().await;
    let replacement = match revoke {
      Revoke::Logout => {
        let response = second
          .dispatch(request(Method::POST, "/logout", &original))
          .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
          response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
        );
        None
      }
      Revoke::Rotate => {
        let response = second
          .dispatch(request(Method::POST, "/rotate", &original))
          .await;
        assert_eq!(response.status(), StatusCode::OK);
        Some(cookie(&response))
      }
      Revoke::All => {
        handle.revoke_all();
        None
      }
      Revoke::Matching => {
        handle.revoke_where(|_, data| {
          data.get("user").and_then(serde_json::Value::as_str) == Some("alice")
        });
        None
      }
    };
    pause.release.notify_one();
    replacement
  };
  let (late, replacement) = futures_util::join!(late, invalidate);
  assert_eq!(late.status(), StatusCode::OK);
  assert_eq!(
    body(
      second
        .dispatch(request(Method::GET, "/whoami", &original))
        .await
    )
    .await,
    "anonymous",
    "the old cookie must stay revoked after the late request finishes",
  );
  assert!(
    !late.headers().contains_key("set-cookie"),
    "a stale response must neither mint an authenticated session nor overwrite a rotated cookie",
  );
  if let Some(replacement) = replacement {
    assert_ne!(original, replacement);
    assert_eq!(
      body(
        second
          .dispatch(request(Method::GET, "/whoami", &replacement))
          .await
      )
      .await,
      "alice",
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn logout_rejects_late_session_writes_and_rotations() {
  for rotate_late in [false, true] {
    late_request_cannot_restore_session(Revoke::Logout, rotate_late).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn rotation_rejects_late_session_writes_and_rotations() {
  for rotate_late in [false, true] {
    late_request_cannot_restore_session(Revoke::Rotate, rotate_late).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn revoke_all_rejects_late_session_writes_and_rotations() {
  for rotate_late in [false, true] {
    late_request_cannot_restore_session(Revoke::All, rotate_late).await;
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn revoke_where_rejects_late_session_writes_and_rotations() {
  for rotate_late in [false, true] {
    late_request_cannot_restore_session(Revoke::Matching, rotate_late).await;
  }
}
