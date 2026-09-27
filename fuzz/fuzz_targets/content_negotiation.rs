#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::router::Router;
use tako_rs_extractors::acc_lang::AcceptLanguage;
use tako_rs_extractors::accept::Accept;
use tako_rs_plugins::plugins::compression::CompressionBuilder;

fuzz_target!(|data: &[u8]| {
  let Ok(value) = http::HeaderValue::from_bytes(data) else {
    return;
  };
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  runtime.block_on(async {
    let mut request = http::Request::new(TakoBody::empty());
    request
      .headers_mut()
      .insert(http::header::ACCEPT, value.clone());
    let (mut parts, _) = request.into_parts();
    let accept = Accept::from_request_parts(&mut parts).await.unwrap();
    let _ = accept.accepts("application/json");
    if let Ok(text) = value.to_str() {
      let _ = AcceptLanguage::parse_accept_language(text);
    }
    let mut router = Router::new();
    router.plugin(CompressionBuilder::new().min_size(1).build());
    router.get("/", || async { "compressible response" });
    let mut request = http::Request::new(TakoBody::empty());
    request
      .headers_mut()
      .insert(http::header::ACCEPT_ENCODING, value);
    let _ = router.dispatch(request).await;
  });
});
