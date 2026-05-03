# Streams

> **Status:** scaffold.

- **SSE**: `Sse::events(stream)` + `SseEvent` builder, `keep_alive`,
  `last_event_id` helper. Headers default to no-cache plus
  `X-Accel-Buffering: no` for nginx-fronted deployments.
- **WebSocket**: `TakoWs<H>` with subprotocol negotiation, max frame
  / message sizes, allowed origins, upgrade timeout, keep-alive ping /
  pong policy. `permessage-deflate` is exposed via `WebSocketConfig`.
- **File serving**: `FileStream` with strong ETag, conditional GET,
  precompressed sidecars (`.br` / `.gz`), `ServeDirBuilder` with SPA
  fallback and traversal hardening.
- **WebTransport**: currently raw QUIC; the type is also exported as
  `RawQuicSession`. The W3C WebTransport CONNECT handshake is
  deferred.

> Multipart byteranges and Linux `sendfile(2)` are deferred follow-up
> items.
