#![cfg(any(feature = "tls", feature = "http3", feature = "compio-tls"))]

// `TlsCert::with_client_auth` and the H3 config take these types, so `tako` must name them.
#[test]
fn tls_builder_types_are_nameable() {
  fn _takes(_: tako::ClientAuth, _: Option<tako::ReloadableResolver>, _: tako::H3Congestion) {}
  let _ = tako::build_rustls_server_config;
  let _ = tako::tls::load_certs;
}
