use std::time::Duration;

use bytes::Bytes;
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::DuplexStream;

use super::Config;
use super::Core;
use super::WebSocket;
use crate::ws::Error;
use crate::ws::Message;
use crate::ws::ProtocolError;
use crate::ws::runtime::Shared;
use crate::ws::runtime::lock;
use crate::ws::runtime::shared;
use crate::ws::testing::BINARY;
use crate::ws::testing::CONTINUE_END;
use crate::ws::testing::Counts;
use crate::ws::testing::PING;
use crate::ws::testing::TEXT;
use crate::ws::testing::TEXT_START;
use crate::ws::testing::client_frame;
use crate::ws::testing::upgraded_pair;

async fn socket(capacity: usize) -> (WebSocket, DuplexStream, Counts, Shared<Core>) {
  let (upgraded, client, counts) = upgraded_pair(capacity).await;
  let core = shared(Core::new(upgraded, Config::default()));
  (WebSocket::new(core.clone()), client, counts, core)
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn buffered_frames_cost_one_read() {
  let (mut ws, mut client, counts, _) = socket(1 << 20).await;
  let batch: Vec<u8> = (0..16).flat_map(|_| client_frame(TEXT, b"hello")).collect();
  client.write_all(&batch).await.unwrap();
  for _ in 0..16 {
    assert_eq!(ws.recv().await.unwrap().unwrap(), Message::text("hello"));
  }
  assert_eq!(counts.reads(), 1);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn the_owner_task_queues_until_flush() {
  let (mut ws, _client, counts, core) = socket(1 << 20).await;
  let task = crate::ws::testing::spawn(async move {
    lock(&core).owner.enter();
    ws.send(Message::text("a")).await.unwrap();
    ws.send(Message::text("b")).await.unwrap();
    let queued = counts.writes();
    ws.flush().await.unwrap();
    (queued, counts.writes())
  });
  assert_eq!(task.await.unwrap(), (0, 1));
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn other_tasks_write_through() {
  let (mut ws, mut client, _, _) = socket(1 << 20).await;
  ws.send(Message::text("hi")).await.unwrap();
  let mut frame = [0; 4];
  client.read_exact(&mut frame).await.unwrap();
  assert_eq!(frame, [0x81, 2, b'h', b'i']);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn close_writes_at_once() {
  let (mut ws, mut client, _, _) = socket(1 << 20).await;
  ws.close(None).await.unwrap();
  let mut frame = [0; 2];
  client.read_exact(&mut frame).await.unwrap();
  assert_eq!(frame, [0x88, 0]);
  assert!(matches!(
    ws.send(Message::text("late")).await,
    Err(Error::Closed)
  ));
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn close_after_a_peer_close_flushes_the_reply_and_is_idempotent() {
  let (mut ws, mut client, counts, _) = socket(1 << 20).await;
  client
    .write_all(&client_frame(crate::ws::testing::CLOSE, &[3, 232]))
    .await
    .unwrap();
  assert!(ws.recv().await.unwrap().unwrap().is_close());
  ws.close(None).await.unwrap();
  ws.close(None).await.unwrap();
  let mut reply = [0; 4];
  client.read_exact(&mut reply).await.unwrap();
  assert_eq!(reply, [0x88, 2, 3, 232]);
  assert_eq!(counts.writes(), 1);
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn zero_write_threshold_still_answers_pings() {
  let (upgraded, mut client, _) = upgraded_pair(1024).await;
  let config = Config {
    write_buffer_size: 0,
    ..Config::default()
  };
  let core = shared(Core::new(upgraded, config));
  let mut ws = WebSocket::new(core);
  client.write_all(&client_frame(PING, b"p")).await.unwrap();
  assert!(ws.recv().await.unwrap().unwrap().is_ping());
  ws.flush().await.unwrap();
  let mut pong = [0; 3];
  crate::ws::testing::timeout(Duration::from_millis(100), client.read_exact(&mut pong))
    .await
    .expect("automatic pong reaches a peer with a zero write threshold")
    .unwrap();
  assert_eq!(pong, [0x8a, 1, b'p']);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn every_ping_gets_its_pong_in_order() {
  let (mut ws, mut client, _, _) = socket(1 << 20).await;
  let pings: Vec<u8> = (0..10u8).flat_map(|i| client_frame(PING, &[i])).collect();
  client.write_all(&pings).await.unwrap();
  for i in 0..10u8 {
    assert_eq!(
      ws.recv().await.unwrap().unwrap(),
      Message::Ping(Bytes::from(vec![i]))
    );
  }
  ws.flush().await.unwrap();
  let mut pongs = [0; 30];
  crate::ws::testing::timeout(Duration::from_millis(500), client.read_exact(&mut pongs))
    .await
    .expect("a pong for every ping")
    .unwrap();
  let expected: Vec<u8> = (0..10u8).flat_map(|i| [0x8A, 1, i]).collect();
  assert_eq!(pongs.as_slice(), expected.as_slice());
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_pong_precedes_the_reply_to_the_message_its_ping_interrupted() {
  let (mut ws, mut client, _, _) = socket(1 << 20).await;
  let mut input = client_frame(TEXT_START, b"ab");
  input.extend(client_frame(PING, b"p"));
  input.extend(client_frame(CONTINUE_END, b"cd"));
  client.write_all(&input).await.unwrap();
  assert!(ws.recv().await.unwrap().unwrap().is_ping());
  let message = ws.recv().await.unwrap().unwrap();
  ws.send(message).await.unwrap();
  let mut output = [0; 9];
  client.read_exact(&mut output).await.unwrap();
  assert_eq!(output, [0x8A, 1, b'p', 0x81, 4, b'a', b'b', b'c', b'd']);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_ping_flood_from_a_peer_that_never_reads_stays_bounded() {
  let (mut ws, mut client, _, core) = socket(1024).await;
  let observer = core.clone();
  let reader = crate::ws::testing::spawn(async move { while let Some(Ok(_)) = ws.recv().await {} });
  let ping = client_frame(PING, &[7; 125]);
  for _ in 0..2_000 {
    client.write_all(&ping).await.unwrap();
    crate::ws::testing::yield_now().await;
  }
  let queued = lock(&observer).write_buf.len();
  assert!(queued <= 64 * 1024 + 127, "{queued}");
  crate::ws::testing::cancel(reader).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_writer_and_a_reader_on_two_tasks_both_wake() {
  let (ws, mut client, _, _) = socket(1024).await;
  let (mut sink, mut stream) = ws.split();
  let writer = crate::ws::testing::spawn(async move {
    sink
      .send(Message::Binary(Bytes::from(vec![1; 60_000])))
      .await
  });
  crate::ws::runtime::sleep(Duration::from_millis(10)).await;
  let reader = crate::ws::testing::spawn(async move { stream.next().await.is_none() });
  crate::ws::runtime::sleep(Duration::from_millis(10)).await;
  let mut frame = vec![0; 60_004];
  client.read_exact(&mut frame).await.unwrap();
  crate::ws::testing::timeout(Duration::from_secs(1), writer)
    .await
    .expect("the writer is woken")
    .unwrap()
    .unwrap();
  crate::ws::testing::cancel(reader).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_large_frame_does_not_pin_a_large_read_buffer() {
  let (mut ws, mut client, _, core) = socket(4 << 20).await;
  client
    .write_all(&client_frame(BINARY, &vec![7; 1 << 20]))
    .await
    .unwrap();
  let large = ws.recv().await.unwrap().unwrap();
  assert_eq!(large.into_data().len(), 1 << 20);
  let small = client_frame(BINARY, &[1; 1024]);
  let writer = crate::ws::testing::spawn(async move {
    for _ in 0..2048 {
      client.write_all(&small).await.unwrap();
    }
    client
  });
  for _ in 0..2048 {
    ws.recv().await.unwrap().unwrap();
  }
  writer.await.unwrap();
  let capacity = lock(&core).read_buf.capacity();
  assert!(capacity <= 64 * 1024, "{capacity}");
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_peer_that_never_reads_stops_the_sender() {
  let (mut ws, _client, _, core) = socket(64 * 1024).await;
  let observer = core.clone();
  let mut flood = crate::ws::testing::spawn(async move {
    lock(&core).owner.enter();
    let payload = Bytes::from(vec![0; 1024]);
    for _ in 0..10_000 {
      ws.send(Message::Binary(payload.clone())).await.unwrap();
    }
  });
  assert!(
    crate::ws::testing::timeout(Duration::from_millis(500), &mut flood)
      .await
      .is_err()
  );
  let capacity = lock(&observer).write_buf.capacity();
  assert!(capacity < 256 * 1024, "{capacity}");
  crate::ws::testing::cancel(flood).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_protocol_error_is_reported_once_and_answered() {
  let (mut ws, mut client, _, _) = socket(1 << 20).await;
  client.write_all(&[0x82, 0x00]).await.unwrap();
  assert!(matches!(
    ws.recv().await,
    Some(Err(Error::Protocol(ProtocolError::UnmaskedFrame)))
  ));
  assert!(ws.recv().await.is_none());
  ws.flush().await.unwrap();
  let mut close = [0; 4];
  client.read_exact(&mut close).await.unwrap();
  assert_eq!(close, [0x88, 2, 0x03, 0xEA]);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn eof_without_a_close_frame_is_a_reset() {
  let (mut ws, client, _, _) = socket(1 << 20).await;
  drop(client);
  assert!(matches!(
    ws.recv().await,
    Some(Err(Error::Protocol(
      ProtocolError::ResetWithoutClosingHandshake
    )))
  ));
  assert!(ws.recv().await.is_none());
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_large_announced_frame_grows_the_buffer_with_its_bytes() {
  let (mut ws, mut client, _, core) = socket(1 << 20).await;
  let mut header = client_frame(BINARY, &[]);
  header[1] = 0xFF;
  header.splice(2..2, (16u64 << 20).to_be_bytes());
  client.write_all(&header).await.unwrap();
  client.write_all(&[0; 10]).await.unwrap();
  assert!(
    crate::ws::testing::timeout(Duration::from_millis(100), ws.recv())
      .await
      .is_err()
  );
  let capacity = lock(&core).read_buf.capacity();
  assert!(capacity <= 128 * 1024, "{capacity}");
}
