# Transports overview

> **Status:** scaffold.

Tako runs the same `Router` across multiple transports. The `Server`
builder selects which transport(s) a given listener serves.

| Transport | Spawn | Cargo feature |
|---|---|---|
| HTTP/1.1 | `Server::spawn_http` | (default) |
| HTTP/2 (cleartext / h2c) | `Server::spawn_h2c` | `http2` |
| HTTP/2 (TLS / ALPN) | `Server::spawn_tls` | `tls,http2` |
| HTTP/3 (QUIC) | `Server::spawn_h3` | `http3` |
| Unix domain socket | `Server::spawn_unix_http` | (default) |
| Unix abstract namespace | `Server::spawn_unix_http` (path `@name`) | (default, Linux) |
| vsock | `Server::spawn_vsock_http` | `vsock` (Linux) |
| TCP raw | `Server::spawn_tcp_raw` | (default) |
| UDP raw | `Server::spawn_udp_raw` | (default) |
| PROXY protocol | `Server::spawn_proxy_protocol` | (default) |

A single `ServerConfig` flows into every transport — header read
timeouts, keep-alive, drain timeout, max connections, h2 caps, h3
caps, and PROXY read timeout are all sourced from the same struct.
