#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_extractors::query_multi::QueryMulti;
use tako_rs_extractors::query_multi::QueryMultiOptions;

fuzz_target!(|data: &[u8]| {
  let Ok(query) = std::str::from_utf8(data) else {
    return;
  };
  let Ok(mut request) = http::Request::builder()
    .uri(format!("/?{query}"))
    .body(TakoBody::empty())
  else {
    return;
  };
  request
    .extensions_mut()
    .insert(QueryMultiOptions::default().csv_key("tag"));
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap();
  runtime.block_on(async {
    let _ = QueryMulti::<HashMap<String, Vec<String>>>::from_request(&mut request).await;
  });
});
