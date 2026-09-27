#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_extractors::multipart::TakoMultipart;

fuzz_target!(|data: &[u8]| {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  runtime.block_on(async {
    let mut request = http::Request::builder()
      .header("content-type", "multipart/form-data; boundary=fuzz")
      .body(TakoBody::from(data.to_vec()))
      .unwrap();
    if let Ok(TakoMultipart(mut multipart)) = TakoMultipart::from_request(&mut request).await {
      while let Ok(Some(field)) = multipart.next_field().await {
        let _ = field.bytes().await;
      }
    }
  });
});
