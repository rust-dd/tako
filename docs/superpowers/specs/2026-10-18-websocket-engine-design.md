# tako 2.5.0 WebSocket engine: design

Status: approved section by section in conversation on 2026-10-17/18.

## Goal

Replace `tokio-tungstenite`, `compio-ws` and `tungstenite` with a tako-owned WebSocket
implementation shared by `tako::ws` (Tokio and Compio) and the GraphQL subscription
responder.

- Performance: batch writes per handler wake-up, read into uninitialized buffer capacity,
  drop the `AllowStd` and waker-proxy layers. Target: the HttpArena WebSocket board in
  standard mode, where the handler may not batch by itself.
- Clean API and no legacy: 2.5.0 is a breaking release for WebSocket users (accepted
  explicitly, despite the "2.5.0 is additive" note in the 2.5.0 TODO). No shims, no
  deprecated aliases; everything the new engine makes unnecessary is deleted.

## Non-goals

- permessage-deflate (tungstenite did not support it either).
- WebSocket over HTTP/2 (RFC 8441).
- The client role.
- Downcasting `hyper::upgrade::Upgraded` to the concrete socket. Possible later
  optimization if measurements justify it.
- UTF-8 fail-fast inside a single frame. Autobahn 6.4.3 and 6.4.4 stay NON-STRICT, as today.

## Public API

Everything lives in `tako::ws` (re-exported from `tako_rs_core::ws`), with the same names on
both runtimes. The `ws` feature enables it; with `compio` the Compio implementation is used.

```rust
pub struct WebSocketUpgrade<H, Fut>; // Responder
impl<H, Fut> WebSocketUpgrade<H, Fut>
where
  H: FnOnce(WebSocket) -> Fut + 'static, // + Send on Tokio
  Fut: Future<Output = ()> + 'static,    // + Send on Tokio
{
  pub fn new(request: Request, handler: H) -> Self;
  pub fn protocols<I, S>(self, list: I) -> Self;
  pub fn allowed_origins<I, S>(self, origins: I) -> Self;
  pub fn max_frame_size(self, n: usize) -> Self;    // default 16 MiB
  pub fn max_message_size(self, n: usize) -> Self;  // default 64 MiB
  pub fn read_buffer_size(self, n: usize) -> Self;  // default 4 KiB
  pub fn write_buffer_size(self, n: usize) -> Self; // default 64 KiB, see Writes
  pub fn upgrade_timeout(self, d: Duration) -> Self;
  pub fn max_lifetime(self, d: Duration) -> Self;
}

pub struct WebSocket; // Send on Tokio, !Send on Compio
impl WebSocket {
  pub async fn recv(&mut self) -> Option<Result<Message, Error>>; // None after a clean close
  pub async fn send(&mut self, message: Message) -> Result<(), Error>; // queues, see Writes
  pub async fn flush(&mut self) -> Result<(), Error>; // writes everything now
  pub async fn close(&mut self, frame: Option<CloseFrame>) -> Result<(), Error>;
}
impl Stream for WebSocket { type Item = Result<Message, Error>; }
impl Sink<Message> for WebSocket { type Error = Error; } // poll_flush really writes

pub enum Message {
  Text(Utf8Bytes),
  Binary(Bytes),
  Ping(Bytes),
  Pong(Bytes),
  Close(Option<CloseFrame>),
}
pub struct Utf8Bytes; // Bytes known to be UTF-8: as_str, Deref<Target = str>, From<String>,
                      // From<&'static str>, TryFrom<Bytes>, Display
pub struct CloseFrame { pub code: CloseCode, pub reason: Utf8Bytes }
pub struct CloseCode(u16); // associated consts NORMAL, GOING_AWAY, PROTOCOL_ERROR,
                           // UNSUPPORTED, INVALID_PAYLOAD, POLICY, TOO_BIG, INTERNAL, ...

pub enum Error { Io(io::Error), Protocol(ProtocolError), Closed }
#[non_exhaustive]
pub enum ProtocolError { /* reserved bits, unmasked frame, unknown opcode, fragmented or
  oversized control frame, unexpected or missing continuation, invalid UTF-8, invalid close
  frame, frame or message too large, reset without closing handshake, send after close */ }
```

Inherent `recv`/`send`/`flush`/`close` need no trait import. `Stream` and `Sink` stay for
generic code: `split`, `forward`, `select!` over streams, combinators and the GraphQL driver.
Rust method resolution picks the inherent `send` even when `SinkExt` is imported.

## Architecture

`tako-rs-core/src/ws/`, compiled with the `ws` feature. Every file stays under 400 lines.

| File | Responsibility |
| --- | --- |
| `mod.rs` | Module routing and re-exports |
| `message.rs` | `Message`, `Utf8Bytes`, `CloseFrame`, `CloseCode` |
| `error.rs` | `Error`, `ProtocolError` |
| `frame.rs` | Frame header parse and encode, no IO |
| `mask.rs` | Unmasking in `u64` words |
| `utf8.rs` | Incremental UTF-8 validation across fragments |
| `protocol.rs` | Receive state machine: message assembly, control frames, close states, limits |
| `handshake.rs` | RFC 6455 accept and rejection, subprotocol and origin checks |
| `socket.rs` | `WebSocket` over `hyper::upgrade::Upgraded` |
| `driver.rs` | Future that runs the handler and owns batching and shutdown |
| `runtime.rs` | Runtime shims: shared core type, owner check, spawn, sleep |
| `upgrade.rs` | `WebSocketUpgrade` responder and builder |

`hyper::upgrade::Upgraded` implements the poll-based `hyper::rt::Read` and `hyper::rt::Write`
on both runtimes (the Compio server runs hyper through `cyper-core`), so `socket.rs` and
`driver.rs` are runtime-independent. Only `runtime.rs` differs:

| | Tokio | Compio |
| --- | --- | --- |
| Shared core | `Arc<Mutex<Core>>` (std) | `Rc<RefCell<Core>>` |
| Owner check | `tokio::task::try_id() == owner` | the driver's "polling the handler" flag |
| Spawn | `tokio::spawn` | `compio::runtime::spawn(..).detach()` |
| Sleep | `tokio::time::sleep` | `compio::time::sleep` |

The GraphQL subscription responder (`graphql/subscription.rs`) uses `handshake`, `socket` and
`driver` directly. Its duplicated SHA-1 and base64 handshake is deleted. The generic
`GraphQLWebSocket` driver switches its bounds to `tako::ws::{Message, Error}`.

## Reads

- `read_buf: BytesMut`, initial capacity `read_buffer_size` (4 KiB). Reads go through
  `hyper::rt::ReadBuf::uninit` into the spare capacity, so nothing is zeroed.
- A frame header announcing a larger payload does not reserve the whole frame up front: the
  buffer grows with the bytes that actually arrive (at most 64 KiB past what is buffered per
  read), so a 10-byte header cannot make the server allocate `max_frame_size`.
- `recv`/`poll_next` decode every complete frame already buffered before touching the socket,
  so 16 pipelined frames cost one read.
- Payloads of up to 4 KiB are copied out and unmasked in one pass, so a stored small message
  never pins the read buffer and the buffer stays reusable. Larger payloads are split off
  without copying and unmasked in place.

Protocol rules follow RFC 6455 and tungstenite's observable behavior:

| Case | Behavior |
| --- | --- |
| RSV bit set, unmasked client frame, unknown opcode | protocol error |
| Control frame | FIN required, payload at most 125 bytes |
| Continuation without a message, new message inside a fragmented one | protocol error |
| `max_frame_size` | checked on the header, before the payload is read |
| `max_message_size` | checked while fragments accumulate |
| Text | single frame validated at once; fragments validated incrementally, carrying a truncated trailing sequence |
| Ping | queue one Pong (a newer Ping replaces a pending one) and yield `Message::Ping` |
| Pong | yield `Message::Pong` |
| Close | reply with the same code (1002 for a code that may not appear on the wire), yield `Message::Close`; the next `recv` returns `None`. A reply to our own Close completes the handshake |
| EOF | `None` after a closing handshake, `ProtocolError::ResetWithoutClosingHandshake` otherwise |

A protocol error queues a Close frame with its code (1002 protocol, 1007 invalid payload,
1009 too big) before the connection ends. If Autobahn reports WRONG_CODE for any case, fall
back to tungstenite's behavior of ending without a Close frame.

## Writes and batching

`send` encodes the frame (unmasked, server role) into `write_buf` and returns. Frames go out
in send order. Automatic Pong and Close replies are appended to the same buffer in arrival
order.

`write_buf` reaches the socket at these points:

1. After every poll of the handler future, the driver writes everything in one call.
2. `recv`, before returning `Pending` because no input is buffered, writes what is queued.
3. When `write_buf` grows past `write_buffer_size` (64 KiB), `send` writes through.
4. `flush` and `close` write immediately and wait until the buffer is empty.

Queuing applies only inside the driver's own task (owner check in `runtime.rs`). Used from
another task, `send` writes through.

The `Sink` impl keeps the `Sink` contract: `poll_flush` always writes to the socket. Only the
inherent `send` defers. Generic `Sink` code therefore gets one real flush per call; `forward`
still batches because it flushes only when its input is pending.

Backpressure: past `write_buffer_size` with the socket full, `send` and `poll_ready` return
`Pending`.

## Driver lifecycle

The driver future owns the handler future and an `Rc`/`Arc` handle to the shared core.

| Event | Driver action |
| --- | --- |
| Handler returns, no `WebSocket` handle left | queue Close(1000) if the connection is open |
| Handler returns, a handle still lives (moved into another task, split halves) | wait until the last handle drops (its `Drop` wakes the driver), then as above |
| Handler panics (caught with `catch_unwind`) | queue Close(1011) |
| `max_lifetime` expires | drop the handler, queue Close(1001); remaining handles get `Error::Closed` |
| Protocol error during a read | Close with the error's code is already queued |

After any of these, the driver writes out `write_buf` for at most 5 seconds (a peer that does
not read cannot hold the task), calls `poll_shutdown` and finishes. Data queued by `send` is
never silently dropped while the socket accepts writes.

## Defaults

| Setting | Default |
| --- | --- |
| `read_buffer_size` | 4 KiB |
| `write_buffer_size` | 64 KiB |
| `max_frame_size` | 16 MiB |
| `max_message_size` | 64 MiB |
| Copy threshold for small payloads | 4 KiB |
| Close timeout | 5 s |

## Deletions

- `tako-rs-streams/src/ws.rs`, `ws_compio.rs`, `ws_handshake.rs`, and the streams features
  `ws` and `compio-ws`.
- The `tako-rs` feature `compio-ws` (`ws` together with `compio` selects the Compio
  implementation) and the `tako::ws_compio` module.
- `TakoWs`, `TakoWsCompio`, `CompioWebSocket`, `UpgradedStream`, `WsKeepAlive` and
  `TakoWs::keep_alive` (closes TODO item M03 for WebSocket).
- Dependencies: `tokio-tungstenite` (workspace, core, streams, examples) and compio's `ws`
  feature.
- `examples/websocket-http2` is renamed to `examples/websocket-tls`: it serves wss over
  HTTP/1.1, and tako does not implement RFC 8441.

## Testing

- Unit, no IO: frame header lengths (7, 16, 64 bit), masked and unmasked frames, truncated
  headers, `max_frame_size` on the header, RSV and opcode rules, encode round trip; mask
  against a byte-wise reference for lengths 0 to 64 and every alignment; UTF-8 splits at every
  boundary, invalid sequences found in the fragment that makes them invalid, truncated end;
  protocol fragmentation, control frames between fragments, Ping/Pong replacement, both close
  directions, invalid close code (1002), one-byte close payload, invalid close reason (1007),
  message size across fragments, the queued Close code for each protocol error.
- Socket and driver, in-memory IO with a write counter: 16 pipelined frames produce one write;
  `flush` writes at once; `send` from another task writes through; `write_buffer_size` forces
  a write; output queued before a timer await is written before the timer fires; handler end,
  panic and `max_lifetime` send 1000, 1011 and 1001; the 5 s close bound with a peer that does
  not read (`tokio::time::pause`); `poll_ready` returns `Pending` when the socket is full.
- Integration (`tako-rs/tests`): handshake and lease tests on the new types; TCP echo on the
  per-thread and multi-thread servers (text, binary, ping, close, pipelined); a GraphQL
  subscription smoke test (`connection_init` to `connection_ack`); the same under Compio.
- Autobahn: `ci/autobahn` for Tokio and a new `examples/autobahn-echo-compio` for Compio,
  `run.sh` parameterized by example and a Compio CI job. Pass criterion: 247/247 with no case
  behaving worse than 2.4.2.
- Fuzz: a `ws_frame` target in `fuzz/` feeding arbitrary bytes to the protocol layer: no
  panics, memory bounded by the limits.
- Benchmark (Linux VM harness, 2.4.2 against 2.5.0): pipelined at 512 and 4,096 connections at
  least 5 times faster; echo at 512, 4,096 and 16,384 and short-lived at 512 and 4,096 not
  slower; resident memory not higher.

## Migration and docs

- 2.5.0 release notes get a "Breaking: WebSocket" section with a before and after example.
- `website/content/docs/reference/migration.mdx` gets a 2.5 section. `transports/websocket.mdx`,
  `streams.mdx`, `transports/index.mdx`, `tutorials/realtime-chat.mdx` and
  `getting-started/coming-from-axum.mdx` move to the new API.
- Examples `websocket`, `websocket-compio`, `websocket-tls`, `autobahn-echo` and
  `graphql-generic-ws` use the new API with no tungstenite imports.

## Phases

1. Core types, IO-free layers, Tokio socket and driver, `WebSocketUpgrade`, GraphQL.
2. Compio runtime shims and tests.
3. Examples, docs, CI (Autobahn for both runtimes, fuzz target), deletions.
4. Benchmarks and Autobahn against 2.4.2.
5. Release.
