use std::hint::black_box;
use std::time::Duration;
use std::time::Instant;

use bytes::Bytes;

use super::Config;
use super::Core;
use super::WebSocket;
use crate::ws::runtime::lock;
use crate::ws::runtime::shared;
use crate::ws::testing::BINARY;
use crate::ws::testing::client_frame;
use crate::ws::testing::upgraded_pair;

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
#[ignore = "manual throughput measurement; run optimized and without concurrent benchmarks"]
async fn buffered_echo_throughput() {
  let (upgraded, _client, _) = upgraded_pair(1 << 20).await;
  let core = shared(Core::new(upgraded, Config::default()));
  let mut ws = WebSocket::new(core.clone());
  crate::ws::testing::spawn(async move {
    lock(&core).owner.enter();
    for (payload_len, messages) in [(5, 4096), (64, 512), (1024, 32)] {
      let payload = Bytes::from(vec![7; payload_len]);
      let batch = client_frame(BINARY, &payload).repeat(messages);
      let mut elapsed = Duration::ZERO;
      let batches = 2_000;
      for _ in 0..batches {
        lock(&core).read_buf.extend_from_slice(&batch);
        let start = Instant::now();
        for _ in 0..messages {
          let message = ws.recv().await.unwrap().unwrap();
          ws.send(black_box(message)).await.unwrap();
        }
        elapsed += start.elapsed();
        let mut state = lock(&core);
        black_box(&state.write_buf);
        assert!(state.read_buf.is_empty());
        state.write_buf.clear();
      }
      let total = messages * batches;
      eprintln!(
        "buffered echo payload={payload_len} messages={total} elapsed_ns={} messages_per_sec={:.0}",
        elapsed.as_nanos(),
        total as f64 / elapsed.as_secs_f64()
      );
    }
  })
  .await
  .unwrap();
}
