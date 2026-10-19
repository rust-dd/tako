#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
  tako_rs_core::ws::fuzz_decode(data);
});
