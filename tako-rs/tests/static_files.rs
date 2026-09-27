use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use http::Method;
use http::StatusCode;
use http::header;
use http_body_util::BodyExt;
use tako::body::TakoBody;
#[cfg(feature = "file-stream")]
use tako::responder::Responder;
use tako::r#static::PrecompressedPolicy;
use tako::r#static::ServeDir;
use tako::r#static::ServeFile;
use tako::types::Request;

struct Fixture(PathBuf);
impl Fixture {
  fn new() -> Self {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
      "tako-static-{}-{}-{}",
      NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      std::process::id(),
      SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }
  fn write(&self, path: &str, contents: &[u8]) {
    std::fs::write(self.0.join(path), contents).unwrap();
  }
}
impl Drop for Fixture {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

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

fn request(path: &str, headers: &[(&str, &str)]) -> Request {
  let mut builder = http::Request::builder().uri(path);
  for (name, value) in headers {
    builder = builder.header(*name, *value);
  }
  builder.body(TakoBody::empty()).unwrap()
}

#[test]
fn decoded_paths_and_dotfile_policy_preserve_the_root_boundary() {
  run(async {
    let fixture = Fixture::new();
    fixture.write("my fájl.txt", b"unicode");
    fixture.write(".env", b"secret");
    fixture.write("index.html", b"fallback");
    let dir = ServeDir::builder(&fixture.0)
      .fallback(fixture.0.join("index.html"))
      .build();
    let response = dir.handle(request("/my%20f%C3%A1jl.txt", &[])).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      "unicode"
    );
    for path in [
      "/.env",
      "/%2eenv",
      "/../outside",
      "/%2e%2e/outside",
      "/%2e%2e%2foutside",
      "/%5c..%5coutside",
      "/%00file",
      "/%ff",
    ] {
      assert_eq!(
        dir.handle(request(path, &[])).await.status(),
        StatusCode::NOT_FOUND,
        "{path}"
      );
    }
    assert_eq!(
      dir
        .handle(request("/missing", &[]))
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes(),
      "fallback"
    );
    let permitted = ServeDir::builder(&fixture.0).allow_dotfiles(true).build();
    assert_eq!(
      permitted.handle(request("/.env", &[])).await.status(),
      StatusCode::OK
    );
  });
}

#[cfg(unix)]
#[test]
fn file_index_and_sidecar_symlinks_cannot_escape_the_root() {
  run(async {
    use std::os::unix::fs::symlink;
    let root = Fixture::new();
    let outside = Fixture::new();
    outside.write("private", b"secret");
    root.write("asset.txt", b"public");
    symlink(outside.0.join("private"), root.0.join("leak")).unwrap();
    symlink(outside.0.join("private"), root.0.join("index.html")).unwrap();
    symlink(outside.0.join("private"), root.0.join("asset.txt.gz")).unwrap();
    root.write(".secret", b"hidden");
    symlink(root.0.join(".secret"), root.0.join("alias")).unwrap();
    let dir = ServeDir::builder(&root.0)
      .precompressed(PrecompressedPolicy::both())
      .build();
    for path in ["/", "/leak", "/alias"] {
      assert_eq!(
        dir.handle(request(path, &[])).await.status(),
        StatusCode::NOT_FOUND
      );
    }
    let response = dir
      .handle(request("/asset.txt", &[("accept-encoding", "gzip")]))
      .await;
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      "public"
    );
  });
}

#[test]
fn cache_validators_precede_ranges_and_head_has_no_body() {
  run(async {
    let fixture = Fixture::new();
    fixture.write("asset.txt", b"0123456789");
    let dir = ServeDir::builder(&fixture.0)
      .cache_control("public, max-age=60".parse().unwrap())
      .build();
    let response = dir.handle(request("/asset.txt", &[])).await;
    let etag = response.headers()[header::ETAG]
      .to_str()
      .unwrap()
      .to_owned();
    let modified = response.headers()[header::LAST_MODIFIED]
      .to_str()
      .unwrap()
      .to_owned();
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
    for conditional in [
      ("if-none-match", etag.as_str()),
      ("if-modified-since", modified.as_str()),
    ] {
      let response = dir
        .handle(request(
          "/asset.txt",
          &[conditional, ("range", "bytes=2-4")],
        ))
        .await;
      assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
      assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=60"
      );
      assert_eq!(response.headers()[header::ETAG], etag);
      assert!(
        response
          .into_body()
          .collect()
          .await
          .unwrap()
          .to_bytes()
          .is_empty()
      );
    }
    let response = dir
      .handle(request(
        "/asset.txt",
        &[
          ("if-none-match", "\"other\""),
          ("if-modified-since", &modified),
        ],
      ))
      .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      dir
        .handle(request("/asset.txt", &[("if-match", &etag)]))
        .await
        .status(),
      StatusCode::PRECONDITION_FAILED
    );
    let mut head = request("/asset.txt", &[("range", "bytes=2-4")]);
    *head.method_mut() = Method::HEAD;
    let response = dir.handle(head).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
    assert!(
      response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .is_empty()
    );
    let mut post = request("/asset.txt", &[]);
    *post.method_mut() = Method::POST;
    let response = dir.handle(post).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
  });
}

#[test]
fn byte_ranges_stream_only_the_requested_bytes() {
  run(async {
    let fixture = Fixture::new();
    fixture.write("asset.txt", b"0123456789");
    let file = ServeFile::builder(fixture.0.join("asset.txt")).build();
    for (range, expected, content_range) in [
      ("bytes=0-0", "0", "bytes 0-0/10"),
      ("bytes=4-", "456789", "bytes 4-9/10"),
      ("bytes=-3", "789", "bytes 7-9/10"),
      ("bytes=7-99", "789", "bytes 7-9/10"),
    ] {
      let response = file.handle(request("/", &[("range", range)])).await;
      assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
      assert_eq!(response.headers()[header::CONTENT_RANGE], content_range);
      assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        expected
      );
    }
    let response = file.handle(request("/", &[("range", "bytes=10-")])).await;
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
    for headers in [
      vec![("range", "bytes=0-0,2-2")],
      vec![("range", "bytes=1-2"), ("if-range", "W/\"stale\"")],
      vec![("range", "items=1-2")],
    ] {
      assert_eq!(
        file.handle(request("/", &headers)).await.status(),
        StatusCode::OK
      );
    }
    fixture.write("empty", b"");
    let empty = ServeFile::builder(fixture.0.join("empty")).build();
    assert_eq!(
      empty
        .handle(request("/", &[("range", "bytes=-1")]))
        .await
        .status(),
      StatusCode::OK
    );
  });
}

#[test]
fn precompressed_negotiation_varies_identity_and_uses_distinct_validators() {
  run(async {
    let fixture = Fixture::new();
    fixture.write("asset.txt", b"identity");
    fixture.write("asset.txt.gz", b"gzip-sidecar");
    fixture.write("asset.txt.br", b"brotli-sidecar");
    let dir = ServeDir::builder(&fixture.0)
      .precompressed(PrecompressedPolicy::both())
      .build();
    let plain = dir.handle(request("/asset.txt", &[])).await;
    assert_eq!(plain.headers()[header::VARY], "Accept-Encoding");
    let identity_etag = plain.headers()[header::ETAG].clone();
    let gzip = dir
      .handle(request(
        "/asset.txt",
        &[("accept-encoding", "br;q=0.1, gzip;q=0.9")],
      ))
      .await;
    assert_eq!(gzip.headers()[header::CONTENT_ENCODING], "gzip");
    assert_ne!(gzip.headers()[header::ETAG], identity_etag);
    assert_eq!(gzip.headers()[header::CONTENT_TYPE], "text/plain");
    let identity = dir
      .handle(request(
        "/asset.txt",
        &[("accept-encoding", "br;q=0.0, gzip;q=0.000, *;q=1")],
      ))
      .await;
    assert!(!identity.headers().contains_key(header::CONTENT_ENCODING));
    let range = dir
      .handle(request(
        "/asset.txt",
        &[("accept-encoding", "gzip"), ("range", "bytes=0-3")],
      ))
      .await;
    assert_eq!(range.headers()[header::CONTENT_ENCODING], "gzip");
    assert_eq!(
      range.into_body().collect().await.unwrap().to_bytes(),
      "gzip"
    );
  });
}

#[test]
fn large_files_have_bounded_frames_and_can_be_router_handlers() {
  run(async {
    let fixture = Fixture::new();
    std::fs::File::create(fixture.0.join("large.bin"))
      .unwrap()
      .set_len(16 * 1024 * 1024)
      .unwrap();
    let dir = ServeDir::builder(&fixture.0).build();
    let mut router = tako::router::Router::new();
    router.get("/large.bin", move |request: Request| {
      let dir = dir.clone();
      async move { dir.handle(request).await }
    });
    let mut response = router.dispatch(request("/large.bin", &[])).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "16777216");
    let chunk = response
      .body_mut()
      .frame()
      .await
      .unwrap()
      .unwrap()
      .into_data()
      .unwrap();
    assert!(!chunk.is_empty());
    assert!(chunk.len() <= 64 * 1024);
    #[cfg(feature = "file-stream")]
    {
      let mut response = tako::file_stream::FileStream::from_path(fixture.0.join("large.bin"))
        .await
        .unwrap()
        .into_response();
      let chunk = response
        .body_mut()
        .frame()
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
      assert!(chunk.len() <= 64 * 1024);
      let response =
        tako::file_stream::FileStream::try_range_response(fixture.0.join("large.bin"), 0, 0)
          .await
          .unwrap();
      assert_eq!(response.headers()[header::CONTENT_LENGTH], "1");
      assert_eq!(
        response
          .into_body()
          .collect()
          .await
          .unwrap()
          .to_bytes()
          .len(),
        1
      );
    }
  });
}
