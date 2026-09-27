#![cfg(feature = "plugins")]

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::plugins::compression::CompressionBuilder;
use tako::plugins::rate_limiter::Algorithm;
use tako::plugins::rate_limiter::RateLimiterBuilder;
use tako::router::Router;
use tako::types::Request;

fn request(encoding: &str) -> Request {
  http::Request::builder()
    .uri("/")
    .header("accept-encoding", encoding)
    .body(TakoBody::empty())
    .unwrap()
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn both_rate_limit_algorithms_start_with_a_full_burst() {
  for algorithm in [Algorithm::TokenBucket, Algorithm::Gcra] {
    let mut router = Router::new();
    router.get("/", || async { "ok" });
    router.plugin(
      RateLimiterBuilder::new()
        .max_requests(10)
        .refill_rate(10)
        .refill_interval_ms(60_000)
        .algorithm(algorithm)
        .key_fn(|_| Some("client".into()))
        .build(),
    );
    router.setup_plugins_once().unwrap();
    for _ in 0..10 {
      assert_eq!(router.dispatch(request("")).await.status(), StatusCode::OK);
    }
    let response = router.dispatch(request("")).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["ratelimit-remaining"], "0");
    assert!(
      response.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse::<u64>()
        .unwrap()
        > 0
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn buffered_compression_leaves_open_ended_streams_readable() {
  for content_type in ["text/event-stream", "application/x-ndjson", "text/plain"] {
    for encoding in ["gzip", "identity"] {
      let mut router = Router::new();
      router.get("/", move || async move {
        let stream = futures_util::stream::once(async {
          Ok::<_, std::io::Error>(Bytes::from_static(b"first frame\n"))
        })
        .chain(futures_util::stream::pending());
        http::Response::builder()
          .header("content-type", content_type)
          .body(TakoBody::from_stream(stream))
          .unwrap()
      });
      router.plugin(CompressionBuilder::new().build());
      router.setup_plugins_once().unwrap();
      let dispatch = std::pin::pin!(router.dispatch(request(encoding)));
      #[cfg(not(feature = "compio"))]
      let timeout = std::pin::pin!(tokio::time::sleep(Duration::from_secs(1)));
      #[cfg(feature = "compio")]
      let timeout = std::pin::pin!(compio::time::sleep(Duration::from_secs(1)));
      let mut response = match futures_util::future::select(dispatch, timeout).await {
        futures_util::future::Either::Left((response, _)) => response,
        futures_util::future::Either::Right(_) => panic!("compression buffered an open stream"),
      };
      assert!(!response.headers().contains_key("content-encoding"));
      assert_eq!(
        response
          .body_mut()
          .frame()
          .await
          .unwrap()
          .unwrap()
          .into_data()
          .unwrap(),
        "first frame\n"
      );
    }
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn compression_preserves_range_responses() {
  for stream in [false, true] {
    for (status, content_range) in [
      (StatusCode::PARTIAL_CONTENT, true),
      (StatusCode::PARTIAL_CONTENT, false),
      (StatusCode::OK, true),
    ] {
      let mut router = Router::new();
      router.get("/", move || async move {
        let mut response = http::Response::builder()
          .status(status)
          .header("content-type", "text/plain")
          .header("accept-ranges", "bytes")
          .header("content-length", "5");
        if content_range {
          response = response.header("content-range", "bytes 0-4/10");
        }
        response.body(TakoBody::from("hello")).unwrap()
      });
      router.plugin(
        CompressionBuilder::new()
          .enable_stream(stream)
          .min_size(0)
          .build(),
      );
      router.setup_plugins_once().unwrap();
      let response = router.dispatch(request("gzip")).await;
      assert_eq!(response.status(), status);
      assert!(!response.headers().contains_key("content-encoding"));
      assert_eq!(response.headers()["accept-ranges"], "bytes");
      assert_eq!(response.headers()["content-length"], "5");
      assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "hello"
      );
    }
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn compressed_representations_no_longer_advertise_byte_ranges() {
  for stream in [false, true] {
    let mut router = Router::new();
    router.get("/", || async {
      http::Response::builder()
        .header("content-type", "text/plain")
        .header("accept-ranges", "bytes")
        .body(TakoBody::from("hello"))
        .unwrap()
    });
    router.plugin(
      CompressionBuilder::new()
        .enable_stream(stream)
        .min_size(0)
        .build(),
    );
    router.setup_plugins_once().unwrap();
    let response = router.dispatch(request("gzip")).await;
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert!(!response.headers().contains_key("accept-ranges"));
    assert!(!response.headers().contains_key("content-length"));
    assert_eq!(response.headers()["vary"], "Accept-Encoding");
  }
}
