#![no_main]

//! Fuzz target: path-params deserializer.
//!
//! Feeds arbitrary `(name, value)` slot pairs into the params deserializer
//! through the public `Params::<T>::from_request` extractor. The
//! deserializer must never panic on malformed input — it should return
//! `Err(ParamsError::DeserializationError(...))` instead.

use http::Method;
use http::Request;
use libfuzzer_sys::arbitrary::Arbitrary;
use libfuzzer_sys::arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use serde::Deserialize;
use smallvec::SmallVec;
use tako_core::body::TakoBody;
use tako_core::extractors::FromRequest;
use tako_core::extractors::params::Params;
use tako_core::extractors::params::PathParams;

#[derive(Debug, Deserialize)]
struct Pair {
  #[allow(dead_code)]
  id: u64,
  #[allow(dead_code)]
  name: String,
}

#[derive(Debug, Deserialize)]
struct Optional {
  #[allow(dead_code)]
  id: Option<u64>,
  #[allow(dead_code)]
  name: Option<String>,
}

fn make_request(slots: SmallVec<[(String, String); 4]>) -> Request<TakoBody> {
  let mut req = Request::builder()
    .method(Method::GET)
    .uri("/")
    .body(TakoBody::empty())
    .expect("build request");
  req.extensions_mut().insert(PathParams(slots));
  req
}

fuzz_target!(|raw: &[u8]| {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .expect("build runtime");
  runtime.block_on(async {
    let mut u = Unstructured::new(raw);
    let count = u8::arbitrary(&mut u).unwrap_or(0).min(16) as usize;
    let mut slots: SmallVec<[(String, String); 4]> = SmallVec::with_capacity(count);
    for _ in 0..count {
      let key = String::arbitrary(&mut u).unwrap_or_default();
      let value = String::arbitrary(&mut u).unwrap_or_default();
      slots.push((key, value));
    }

    // Each shape is exercised in isolation so the slot list survives.
    {
      let mut req = make_request(slots.clone());
      let _ = Params::<Pair>::from_request(&mut req).await;
    }
    {
      let mut req = make_request(slots.clone());
      let _ = Params::<Optional>::from_request(&mut req).await;
    }
    {
      let mut req = make_request(slots.clone());
      let _ = Params::<Vec<String>>::from_request(&mut req).await;
    }
    {
      let mut req = make_request(slots.clone());
      let _ = Params::<(String, String)>::from_request(&mut req).await;
    }
    {
      let mut req = make_request(slots);
      let _ = Params::<u64>::from_request(&mut req).await;
    }
  });
});
