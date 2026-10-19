# WebSocket engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace tokio-tungstenite, compio-ws and tungstenite with a tako-owned WebSocket engine in `tako_rs_core::ws`, exposed as `tako::ws` on Tokio and Compio and used by GraphQL subscriptions.

**Architecture:** IO-free layers (`frame`, `mask`, `utf8`, `receiver`, `protocol`) decode and encode frames on `BytesMut`. `socket.rs` runs them over `hyper::upgrade::Upgraded` (poll-based `hyper::rt::Read`/`Write` on both runtimes). `driver.rs` runs the handler future, writes queued output once per handler wake-up and owns the close lifecycle. `runtime.rs` holds the only runtime-specific code.

**Tech Stack:** Rust 2024, MSRV 1.95, hyper 1.x, bytes, futures-util, tokio (default runtime), compio (feature `compio`).

**Spec:** `docs/superpowers/specs/2026-10-18-websocket-engine-design.md`

Execution note: this plan pins files, interfaces, tests and algorithms. Implementation code is written once, directly into the files, test first.

## Global Constraints

- Edition 2024, MSRV 1.95 (`rust-version` in the workspace manifest).
- No WebSocket crate in the dependency tree of any workspace crate or example: `tokio-tungstenite`, `tungstenite`, compio's `ws` feature are removed.
- No new dependencies. Allowed in `tako-rs-core`: bytes, futures-util, http, hyper, pin-project-lite, sha1, base64, url, tokio, compio (feature `compio`).
- Public names: `tako::ws::{WebSocketUpgrade, WebSocket, Message, Utf8Bytes, CloseFrame, CloseCode, Error, ProtocolError}`. `ProtocolError` is `#[non_exhaustive]`.
- Defaults: `read_buffer_size` 4 KiB, `write_buffer_size` 64 KiB, `max_frame_size` 16 MiB, `max_message_size` 64 MiB, small-payload copy threshold 4 KiB, close timeout 5 s.
- Files at most 600 lines, target under 400. Comments: one or two lines, only for a non-obvious why. No banners.
- `cargo +nightly fmt --all -- --check`, `cargo clippy --workspace --no-deps --tests --examples [-- default | tokio-rich | --all-features] -- -D warnings` (pedantic is on workspace-wide).
- Tokio-only code and tests are gated `not(feature = "compio")`, Compio code `feature = "compio"`, matching the existing pattern.
- Commits end with `Claude-Session: https://claude.ai/code/session_01XWh5QR83AkTWuNCuh2Z47g`. Commit only when the user asks.

## Review Focus

1. A handler that calls `send` in a tight loop without any yielding await: memory must stay near `write_buffer_size` plus the socket buffer, and `send` must eventually return `Pending` against a peer that does not read. Test in Task 8.
2. A 10-byte frame header announcing 16 MiB, followed by nothing: the server must not allocate the announced size. Test in Task 5.
3. A Ping flood from a peer that never reads: at most one Pong may be pending. Test in Task 6.
4. A handler that moves the `WebSocket` into a spawned task (or splits it) and returns at once: the connection must stay open until the last handle drops. Test in Task 9.
5. A text message split into one-byte fragments, with multi-byte characters cut across fragments: valid input passes, an invalid byte fails in the fragment that carries it. Test in Task 5.

---

### Task 1: Feature wiring, module skeleton, message and error types

**Files:**
- Modify: `tako-rs-core/Cargo.toml` (feature `ws = ["tako-rs/ws", "hyper/http1", "hyper/server"]`, drop `dep:tokio-tungstenite` from it; keep the dependency entry for GraphQL until Task 11)
- Modify: `tako-rs-core/src/lib.rs` (`#[cfg(feature = "ws")] pub mod ws;`)
- Create: `tako-rs-core/src/ws.rs` (module root: `mod error; mod message;` plus later modules; `pub use` of public types)
- Create: `tako-rs-core/src/ws/message.rs`, `tako-rs-core/src/ws/error.rs`

**Interfaces (produced):**
```rust
// message.rs
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Utf8Bytes(Bytes);
impl Utf8Bytes {
  pub const fn from_static(s: &'static str) -> Self;
  pub fn as_str(&self) -> &str;
  pub(crate) unsafe fn from_bytes_unchecked(bytes: Bytes) -> Self;
}
impl Deref<Target = str>, AsRef<str>, AsRef<[u8]>, Display, Debug (as str),
     From<String>, From<&'static str>, TryFrom<Bytes, Error = std::str::Utf8Error>,
     From<Utf8Bytes> for Bytes, PartialEq<str>, PartialEq<&str>

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CloseCode(u16);
impl CloseCode {
  pub const NORMAL: Self; // 1000
  pub const GOING_AWAY: Self; // 1001
  pub const PROTOCOL_ERROR: Self; // 1002
  pub const UNSUPPORTED: Self; // 1003
  pub const INVALID_PAYLOAD: Self; // 1007
  pub const POLICY: Self; // 1008
  pub const TOO_BIG: Self; // 1009
  pub const EXTENSION: Self; // 1010
  pub const INTERNAL: Self; // 1011
  pub const fn new(code: u16) -> Self;
  pub const fn as_u16(self) -> u16;
  /// Codes a peer may send: 1000-1003, 1007-1014, 3000-4999.
  pub const fn is_allowed(self) -> bool;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseFrame { pub code: CloseCode, pub reason: Utf8Bytes }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message { Text(Utf8Bytes), Binary(Bytes), Ping(Bytes), Pong(Bytes), Close(Option<CloseFrame>) }
impl Message {
  pub fn text(text: impl Into<Utf8Bytes>) -> Self;
  pub fn binary(data: impl Into<Bytes>) -> Self;
  pub fn is_text(&self) -> bool; is_binary; is_ping; is_pong; is_close;
  pub fn into_data(self) -> Bytes; // close: reason bytes, or empty
}
impl From<String>, From<&'static str> (Text); From<Bytes>, From<Vec<u8>> (Binary)

// error.rs
#[derive(Debug)]
pub enum Error { Io(std::io::Error), Protocol(ProtocolError), Closed }
impl Display, std::error::Error (source: Io, Protocol), From<std::io::Error>, From<ProtocolError>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolError {
  ReservedBits, UnmaskedFrame, UnknownOpcode(u8), FragmentedControlFrame, ControlFrameTooLarge,
  UnexpectedContinuation, ExpectedContinuation, InvalidUtf8, InvalidCloseFrame,
  FrameTooLarge { size: u64, max: usize }, MessageTooLarge { size: usize, max: usize },
  ResetWithoutClosingHandshake,
}
impl ProtocolError { pub(crate) fn close_code(self) -> CloseCode; } // InvalidUtf8 -> 1007, *TooLarge -> 1009, rest -> 1002
```

- [ ] Write tests in `message.rs`/`error.rs`: `from_static`/`TryFrom<Bytes>` round trip and rejection of `[0xff]`; `CloseCode::is_allowed` for 999, 1000, 1004, 1005, 1006, 1007, 1014, 1015, 2999, 3000, 4999, 5000; `Message::into_data` per variant; `close_code` mapping.
- [ ] Run `cargo test -p tako-rs-core --features ws --lib ws::` and watch them fail to compile.
- [ ] Implement, run again, all pass. `cargo clippy -p tako-rs-core --features ws --tests -- -D warnings` clean.

### Task 2: Masking

**Files:** Create `tako-rs-core/src/ws/mask.rs`.

**Interfaces:** `pub(crate) fn unmask(data: &mut [u8], mask: [u8; 4])`. XOR in place; the body walks `chunks_exact_mut(8)` with `u64::from_ne_bytes`/`to_ne_bytes` and a `u64` built from the mask repeated twice, then finishes the remainder byte-wise with `mask[i & 3]`.

- [ ] Test: for every length 0..=64 and every start offset 0..8 into a 72-byte buffer, compare against a naive `data[i] ^= mask[i % 4]`; applying the mask twice restores the input.
- [ ] Fail, implement, pass.

### Task 3: Incremental UTF-8

**Files:** Create `tako-rs-core/src/ws/utf8.rs`.

**Interfaces:**
```rust
#[derive(Default)]
pub(crate) struct Utf8Validator { tail: [u8; 4], tail_len: u8 }
impl Utf8Validator {
  pub(crate) fn feed(&mut self, chunk: &[u8]) -> Result<(), ProtocolError>; // InvalidUtf8
  pub(crate) fn finish(&self) -> Result<(), ProtocolError>; // a leftover tail is InvalidUtf8
}
```
Algorithm: complete a carried tail first (sequence width from its lead byte: `0xC2..=0xDF` two, `0xE0..=0xEF` three, `0xF0..=0xF4` four; anything else is invalid), validating the joined bytes with `std::str::from_utf8`; a still-incomplete tail must be a valid prefix (`Utf8Error::error_len() == None`). Then `from_utf8(rest)`: `Ok` clears the tail, `Err(e)` with `error_len() == None` stores `rest[e.valid_up_to()..]` (at most 3 bytes), any other `Err` is invalid.

- [ ] Tests: "héllo wörld €𝄞" fed one byte at a time and in every two-way split passes `finish`; `[0xE2, 0x82]` then `finish` fails; `[0xC0, 0x80]` fails on `feed`; `[0xED, 0xA0, 0x80]` (surrogate) fails on `feed` even when split after the first byte; `[0xF4, 0x90]` fails on `feed`.
- [ ] Fail, implement, pass.

### Task 4: Frame header parse and encode

**Files:** Create `tako-rs-core/src/ws/frame.rs`.

**Interfaces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpCode { Continuation, Text, Binary, Close, Ping, Pong }
impl OpCode { pub(crate) fn is_control(self) -> bool; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header { pub fin: bool, pub opcode: OpCode, pub mask: [u8; 4], pub len: u64, pub header_len: usize }
/// `Ok(None)` while `buf` holds less than a whole header.
pub(crate) fn parse_header(buf: &[u8], max_frame_size: usize) -> Result<Option<Header>, ProtocolError>;
pub(crate) fn encode_header(out: &mut BytesMut, fin: bool, opcode: OpCode, len: usize);
```
Rules in `parse_header`: RSV bits -> `ReservedBits`; opcodes 3-7 and 11-15 -> `UnknownOpcode(op)`; mask bit clear -> `UnmaskedFrame`; control with `!fin` -> `FragmentedControlFrame`, with len over 125 -> `ControlFrameTooLarge`; len over `max_frame_size` -> `FrameTooLarge`. Lengths: 0..=125 inline, 126 + u16, 127 + u64 (a set top bit is `FrameTooLarge`). `encode_header` writes server frames: no mask, minimal length encoding.

- [ ] Tests: header lengths 0, 125, 126, 65535, 65536 (encode, then build a masked client frame from it and parse back); every truncated prefix of a valid header returns `Ok(None)`; each rule above returns its error; a header announcing `max_frame_size + 1` fails before any payload exists.
- [ ] Fail, implement, pass.

### Task 5: Receiver (frame assembly)

**Files:** Create `tako-rs-core/src/ws/receiver.rs`.

**Interfaces:**
```rust
pub(crate) struct Limits { pub max_frame_size: usize, pub max_message_size: usize }
pub(crate) enum Frame { Message(Message), Ping(Bytes), Pong(Bytes), Close(Option<CloseFrame>) }
pub(crate) struct Receiver { /* limits, header: Option<Header>, partial: Option<Partial> */ }
impl Receiver {
  pub(crate) fn new(limits: Limits) -> Self;
  /// Next complete frame from `buf`, consuming its bytes. `Ok(None)` when more input is needed.
  pub(crate) fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Frame>, ProtocolError>;
  /// Bytes the pending frame still needs, for sizing the next read (0 when unknown).
  pub(crate) fn wanted(&self, buf: &BytesMut) -> usize;
}
```
Behavior: parse the header once and keep it until the payload is complete. Payloads up to 4 KiB: `Bytes::copy_from_slice` then `unmask`; larger: `buf.split_to(len)`, `unmask` in place, `freeze`. Fragments: `partial` holds the kind (text or binary), a `BytesMut` and a `Utf8Validator`; a continuation without `partial` is `UnexpectedContinuation`, a text or binary frame while `partial` exists is `ExpectedContinuation`; the running size over `max_message_size` is `MessageTooLarge`. Single-frame text uses `std::str::from_utf8`, fragments use `Utf8Validator::feed` per fragment and `finish` on the final one. Close payload: empty is `None`, one byte is `InvalidCloseFrame`, otherwise a code plus a UTF-8 reason (invalid UTF-8 is `InvalidUtf8`). Control frames may arrive between fragments.

- [ ] Tests: single text, binary, ping, pong, close frames; 16 frames in one buffer decode one by one with nothing left; a frame split at every byte position decodes once complete; fragmented text "héllo" in one-byte fragments with a Ping in the middle; an invalid byte in fragment 2 of 3 fails at fragment 2; each fragment-order error; `MessageTooLarge` across fragments; **review focus 2:** a header announcing 16 MiB with 10 payload bytes leaves `buf.capacity()` under 128 KiB after `decode` returns `Ok(None)`; a one-byte close payload fails.
- [ ] Fail, implement, pass.

### Task 6: Connection protocol (close states, control replies, encoding)

**Files:** Create `tako-rs-core/src/ws/protocol.rs`.

**Interfaces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State { Open, CloseSent, CloseReceived, Closed }
pub(crate) struct Protocol { /* receiver, state, pending_pong: Option<Bytes>, pending_close: Option<Option<CloseFrame>> */ }
impl Protocol {
  pub(crate) fn new(limits: Limits) -> Self;
  pub(crate) fn state(&self) -> State;
  /// Next message for the user; queues the automatic Pong and Close replies.
  pub(crate) fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Message>, ProtocolError>;
  /// Encodes a user message; a Close moves the state to CloseSent (or Closed if one was received).
  pub(crate) fn encode(&mut self, message: Message, out: &mut BytesMut) -> Result<(), Error>;
  /// Appends pending control replies to `out`.
  pub(crate) fn drain_control(&mut self, out: &mut BytesMut);
  /// Queues Close with the error's code and marks the connection closed.
  pub(crate) fn fail(&mut self, error: ProtocolError);
  /// The peer closed the transport.
  pub(crate) fn eof(&mut self) -> Result<(), ProtocolError>;
  /// Queues Close(code) when the connection is still open; used by the driver.
  pub(crate) fn close_if_open(&mut self, code: CloseCode);
}
```
Rules: a Ping while `Open` replaces `pending_pong`; Pong is passed through; Close while `Open` queues a reply with the same code (1002 with reason "Protocol violation" for a code that `is_allowed` rejects) and moves to `CloseReceived`; Close while `CloseSent` moves to `Closed`; frames after `CloseReceived`/`Closed` are ignored. `encode` in `CloseReceived` or `Closed`, or after our Close, returns `Error::Closed` (a Close reply drained by `drain_control` is how we answer). `eof` is `Ok` in `CloseReceived`/`Closed`, otherwise `ResetWithoutClosingHandshake`. After `decode` yields a Close, the next `decode` returns `Ok(None)` and `state()` tells the socket to end the stream.

- [ ] Tests: ping then pong reply bytes match `encode_header(Pong) + payload`; **review focus 3:** 1,000 pings before any drain leave exactly one Pong (the last payload); peer close echoes the code; peer close with 1005 on the wire replies 1002; our close then peer close reaches `Closed`; `encode` after close is `Error::Closed`; `fail(InvalidUtf8)` drains a 1007 Close; `eof` in `Open` errors and in `CloseReceived` does not.
- [ ] Fail, implement, pass.

### Task 7: Handshake

**Files:**
- Create `tako-rs-core/src/ws/handshake.rs` from `tako-rs-streams/src/ws_handshake.rs` (functions `accept` and `rejection`, tests) plus `negotiate_subprotocol(headers, &[&'static str]) -> Option<&'static str>` and `origin_allowed(headers, Option<&[String]>) -> bool` with `normalize_origin` and its tests from `tako-rs-streams/src/ws.rs`.

**Interfaces:**
```rust
pub(crate) fn accept(request: &Request) -> Result<String, StatusCode>;
pub(crate) fn rejection(status: StatusCode) -> Response;
pub(crate) fn negotiate_subprotocol(headers: &HeaderMap, protocols: &[&'static str]) -> Option<&'static str>;
pub(crate) fn origin_allowed(headers: &HeaderMap, allowed: Option<&[String]>) -> bool;
```
- [ ] Move the existing tests with the code; they fail until the module is wired, then pass.

### Task 8: Runtime shims and the socket

**Files:** Create `tako-rs-core/src/ws/runtime.rs`, `tako-rs-core/src/ws/socket.rs`.

**Interfaces:**
```rust
// runtime.rs, Tokio half (Compio half in Task 12)
pub(crate) type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;
pub(crate) fn lock<T>(shared: &Shared<T>) -> std::sync::MutexGuard<'_, T>; // poison -> into_inner
#[derive(Default)]
pub(crate) struct Owner(Option<tokio::task::Id>);
impl Owner { pub(crate) fn enter(&mut self); pub(crate) fn leave(&mut self); pub(crate) fn is_current(&self) -> bool; }
pub(crate) fn spawn(future: impl Future<Output = ()> + Send + 'static);
pub(crate) type Sleep = Pin<Box<tokio::time::Sleep>>;
pub(crate) fn sleep(duration: Duration) -> Sleep;

// socket.rs
pub(crate) struct Config { pub limits: Limits, pub read_buffer_size: usize, pub write_buffer_size: usize }
pub(crate) struct Core { io: Upgraded, read_buf: BytesMut, write_buf: BytesMut, protocol: Protocol,
  config: Config, owner: Owner, handles: usize, driver: Option<Waker>, finished: bool }
impl Core {
  pub(crate) fn new(io: Upgraded, config: Config) -> Self;
  pub(crate) fn poll_write_out(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>>;
  pub(crate) fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>>;
  pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Message, Error>>>;
}
pub struct WebSocket { shared: Shared<Core> }
impl WebSocket {
  pub(crate) fn new(shared: Shared<Core>) -> Self; // handles += 1
  pub async fn recv(&mut self) -> Option<Result<Message, Error>>;
  pub async fn send(&mut self, message: Message) -> Result<(), Error>;
  pub async fn flush(&mut self) -> Result<(), Error>;
  pub async fn close(&mut self, frame: Option<CloseFrame>) -> Result<(), Error>;
}
impl Drop for WebSocket // handles -= 1; at 0 wake `driver`
impl Stream for WebSocket; impl Sink<Message> for WebSocket // poll_flush = poll_write_out
```
Reads: `poll_recv` loops `protocol.decode(&mut read_buf)`; on `Ok(None)` it first writes queued output (owner or not), then reserves `max(read_buffer_size, min(wanted, 64 KiB))` of spare capacity and reads through `hyper::rt::ReadBuf::uninit(read_buf.spare_capacity_mut())` and `Upgraded::poll_read`, then `set_len`. Zero bytes calls `protocol.eof()`. A `ProtocolError` calls `protocol.fail`, yields `Some(Err(..))` once, then `None`. A decoded Close yields the message; the following call returns `None`.
Writes: `send` waits on readiness (`write_buf.len() < write_buffer_size`, else `poll_write_out`, `Pending` while still full), then `protocol.encode`; when `owner.is_current()` is false it awaits `poll_write_out`. `poll_write_out` first calls `protocol.drain_control(&mut write_buf)`, then loops `poll_write` and `advance`, then `poll_flush`.

- [ ] Test helper in `socket.rs` tests: start `hyper::server::conn::http1` over one end of `tokio::io::duplex(1 << 20)` wrapped in a write-counting IO, answer the upgrade with 101, take `Upgraded` from `hyper::upgrade::on`, return it with the client end and the counter. The client writes masked frames by hand.
- [ ] Tests: 16 queued frames read by `recv` one by one with the read count at one; `send` from inside a task created by `runtime::spawn` with `Owner` entered for that task does not write until `flush`; `send` from another task writes at once; `flush` writes at once; **review focus 1:** 10,000 sends of 1 KiB to a client that never reads make `send` return `Pending` within a timeout while `write_buf.capacity()` stays under 256 KiB; a protocol error yields one `Err` then `None`; EOF without close yields the reset error.
- [ ] Fail, implement, pass.

### Task 9: Driver

**Files:** Create `tako-rs-core/src/ws/driver.rs`.

**Interfaces:**
```rust
pub(crate) struct Driver<F> { /* shared, handler: Option<Pin<Box<CatchUnwind<AssertUnwindSafe<F>>>>>, lifetime: Option<Sleep>, phase */ }
impl<F: Future<Output = ()>> Driver<F> { pub(crate) fn new(shared: Shared<Core>, handler: F, max_lifetime: Option<Duration>) -> Self; }
impl<F: Future<Output = ()>> Future for Driver<F> { type Output = (); }
```
Phases: `Running` polls the handler between `owner.enter()` and `owner.leave()`, then `poll_write_out` (its `Pending` registers write interest). Handler `Ready(Ok)`: `Detached` while `handles > 0` (store the waker in `core.driver`), then `close_if_open(NORMAL)`. Handler panic: `close_if_open(INTERNAL)`. Lifetime elapsed: drop the handler, `close_if_open(GOING_AWAY)`, mark `finished` so remaining handles get `Error::Closed`. `Closing` writes out and shuts down, bounded by a 5 s `Sleep`.

- [ ] Tests (time paused where timers matter): handler that returns sends Close(1000) and the client sees EOF; handler that panics sends Close(1011); `max_lifetime` 1 s sends Close(1001); the close bound ends the driver after 5 s against a client that never reads; a handler that sends and then awaits a 10 s sleep has its frame on the wire before the sleep ends; **review focus 4:** a handler that moves the socket into `runtime::spawn` and returns keeps the connection open, and the spawned task's echo still works until it drops the socket, after which Close(1000) arrives.
- [ ] Fail, implement, pass.

### Task 10: `WebSocketUpgrade`, re-export, Tokio integration

**Files:**
- Create `tako-rs-core/src/ws/upgrade.rs`.
- Modify `tako-rs/src/lib.rs`: `#[cfg(feature = "ws")] pub use tako_rs_core::ws;`, drop the `ws` and `ws_compio` re-exports from `tako_rs_streams`.
- Modify `tako-rs/Cargo.toml`: `ws = ["tako-rs-core/ws"]`, delete `compio-ws`.
- Delete `tako-rs-streams/src/ws.rs`, `tako-rs-streams/src/ws_handshake.rs`, their `mod` lines in `tako-rs-streams/src/lib.rs`, the streams features `ws`/`compio-ws` and the deps only they used (`tokio-tungstenite`, `sha1`, `base64` if unused elsewhere, `url` if unused elsewhere).
- Modify tests: `tako-rs/tests/websocket_handshake.rs`, `tako-rs/tests/websocket_connection_lease.rs` (new types, `recv`/`send`).
- Create `tako-rs/tests/websocket_echo.rs`.

**Interfaces:**
```rust
pub struct WebSocketUpgrade<H, Fut> { request, handler, protocols: Vec<&'static str>, allowed_origins: Option<Vec<String>>,
  config: Config, upgrade_timeout: Option<Duration>, max_lifetime: Option<Duration>, _fut: PhantomData<fn() -> Fut> }
impl<H, Fut> WebSocketUpgrade<H, Fut> { new, protocols, allowed_origins, max_frame_size, max_message_size,
  read_buffer_size, write_buffer_size, upgrade_timeout, max_lifetime }
impl<H, Fut> Responder for WebSocketUpgrade<H, Fut>
where H: FnOnce(WebSocket) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static
```
`into_response`: `handshake::accept`, origin check (403 "origin not allowed"), subprotocol, 101 response; spawn: await `OnUpgrade` (with `upgrade_timeout`), build `Core` and `WebSocket`, run `Driver::new(shared, handler(ws), max_lifetime)`.

- [ ] Port the handshake and lease tests (they fail to compile against the new names first).
- [ ] `websocket_echo.rs` over real TCP on `Server::spawn_http` and on `spawn_per_thread`: text, binary, ping (expects Pong), 16 pipelined frames echoed in order, client Close answered with the same code and then EOF.
- [ ] `cargo test --workspace --features "ws,per-thread"` passes.

### Task 11: GraphQL subscriptions

**Files:** Modify `tako-rs-core/src/graphql/subscription.rs`, `tako-rs-core/src/graphql/websocket.rs`, `tako-rs-core/Cargo.toml` (remove `tokio-tungstenite` entirely).

- `subscription.rs`: replace the inline SHA-1/base64 accept with `ws::handshake::accept`, keep the graphql-ws subprotocol selection, run the session through `Core`, `WebSocket` and `Driver`; outgoing messages go through `forward` into the sink half so batching applies.
- `websocket.rs`: bounds become `SinkT: Sink<crate::ws::Message>`, `StreamT: Stream<Item = Result<crate::ws::Message, crate::ws::Error>>`.
- [ ] Test (`tako-rs/tests/graphql_ws.rs`, features `ws,async-graphql`): `connection_init` gets `connection_ack` over a real connection.

### Task 12: Compio

**Files:** Modify `tako-rs-core/src/ws/runtime.rs` (Compio half), `tako-rs-core/src/ws/upgrade.rs` (Compio `Responder` bounds without `Send`); delete `tako-rs-streams/src/ws_compio.rs`; modify `examples/websocket-compio`; create `examples/autobahn-echo-compio`.

Compio half: `Shared<T> = Rc<RefCell<T>>`, `lock` is `borrow_mut`, `Owner(bool)` set by `enter`/`leave`, `spawn` is `compio::runtime::spawn(..).detach()`, `Sleep` is `Pin<Box<dyn Future<Output = ()>>>` from `compio::time::sleep`.

- [ ] Test (`tako-rs/tests/websocket_echo_compio.rs`, features `compio,ws`): the echo and close cases of Task 10 on `CompioServer`.

### Task 13: Examples, CI, fuzz, dependency cleanup

**Files:** examples `websocket`, `websocket-http2` renamed to `websocket-tls` (also in the workspace `exclude` list), `autobahn-echo`, `graphql-generic-ws`, `websocket-compio`, `autobahn-echo-compio`; `ci/autobahn/run.sh` (example name as an argument, default `autobahn-echo`); `.github/workflows/ci.yml` (Compio Autobahn job, example list); workspace `Cargo.toml` (remove `tokio-tungstenite`); `fuzz/fuzz_targets/ws_frame.rs` and `fuzz/Cargo.toml`.

- [ ] `rg -n "tungstenite|TakoWs|ws_compio|compio-ws" --glob '!target' --glob '!*.lock'` returns nothing outside the changelog and migration text.
- [ ] Every example builds with `cargo check --manifest-path examples/<name>/Cargo.toml`.
- [ ] `cargo +nightly fuzz build ws_frame` builds.

### Task 14: Docs and migration

**Files:** `website/content/docs/transports/websocket.mdx`, `streams.mdx`, `transports/index.mdx`, `tutorials/realtime-chat.mdx`, `getting-started/coming-from-axum.mdx`, `reference/migration.mdx` (2.5 section: before and after, `TakoWs` to `WebSocketUpgrade`, imports, `send` semantics, `compio-ws` to `compio` + `ws`).

- [ ] Every code block in those pages compiles against the new API (copy into a scratch example and `cargo check`).

### Task 15: Verification against 2.4.2

- [ ] Autobahn (container harness): Tokio and Compio 247/247, no case worse than the 2.4.2 report.
- [ ] Linux VM benchmark, 2.4.2 against 2.5.0, three rounds: pipelined at least 5x, echo and short-lived not slower, RSS not higher.
- [ ] Full gates: nightly fmt, clippy (default, tokio-rich, all-features, Linux target with vsock), `cargo test --workspace` (default, rich, http3, all-features), examples, `cargo deny --all-features check`.
