use std::time::Duration;
#[cfg(feature = "compio")]
use std::time::Instant;

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::DuplexStream;
#[cfg(not(feature = "compio"))]
use tokio::time::Instant;

use super::Driver;
use crate::ws::Message;
use crate::ws::WebSocket;
use crate::ws::runtime::shared;
use crate::ws::socket::Config;
use crate::ws::socket::Core;
use crate::ws::testing::Counts;
use crate::ws::testing::TEXT;
use crate::ws::testing::client_frame;
use crate::ws::testing::upgraded_pair;

async fn run<F, Fut>(
  capacity: usize,
  lifetime: Option<Duration>,
  handler: F,
) -> (DuplexStream, Counts)
where
  F: FnOnce(WebSocket) -> Fut,
  Fut: crate::ws::testing::TaskFuture<Output = ()>,
{
  let (upgraded, client, counts) = upgraded_pair(capacity).await;
  let core = shared(Core::new(upgraded, Config::default()));
  let ws = WebSocket::new(core.clone());
  crate::ws::runtime::spawn(Driver::new(core, handler(ws), lifetime));
  (client, counts)
}

async fn read_close(client: &mut DuplexStream) -> u16 {
  let mut frame = [0; 4];
  client.read_exact(&mut frame).await.unwrap();
  assert_eq!(frame[..2], [0x88, 2]);
  u16::from_be_bytes([frame[2], frame[3]])
}

async fn assert_eof(client: &mut DuplexStream) {
  let mut rest = Vec::new();
  client.read_to_end(&mut rest).await.unwrap();
  assert!(rest.is_empty(), "{rest:?}");
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_finished_handler_closes_with_1000() {
  let (mut client, _) = run(1 << 20, None, |ws| async move { drop(ws) }).await;
  assert_eq!(read_close(&mut client).await, 1000);
  assert_eof(&mut client).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_panicking_handler_closes_with_1011() {
  let (mut client, _) = run(1 << 20, None, |ws| async move {
    let _ws = ws;
    panic!("handler failure under test");
  })
  .await;
  assert_eq!(read_close(&mut client).await, 1011);
  assert_eof(&mut client).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn max_lifetime_closes_with_1001() {
  let lifetime = Some(Duration::from_secs(1));
  let (mut client, _) = run(1 << 20, lifetime, |ws| async move {
    let _ws = ws;
    std::future::pending::<()>().await;
  })
  .await;
  assert_eq!(read_close(&mut client).await, 1001);
  assert_eof(&mut client).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn closing_gives_up_on_a_peer_that_never_reads() {
  let (upgraded, _client, _) = upgraded_pair(1024).await;
  let core = shared(Core::new(upgraded, Config::default()));
  let mut ws = WebSocket::new(core.clone());
  let start = Instant::now();
  let handler = async move {
    ws.send(Message::Binary(Bytes::from(vec![0; 64 * 1024])))
      .await
      .unwrap();
  };
  crate::ws::testing::spawn(Driver::new(core, handler, None))
    .await
    .unwrap();
  let elapsed = start.elapsed();
  assert!(elapsed >= Duration::from_secs(5), "{elapsed:?}");
  assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn queued_output_leaves_before_the_handler_waits() {
  let (mut client, _) = run(1 << 20, None, |mut ws| async move {
    ws.send(Message::text("hi")).await.unwrap();
    crate::ws::runtime::sleep(Duration::from_secs(10)).await;
  })
  .await;
  let mut frame = [0; 4];
  let read = crate::ws::testing::timeout(Duration::from_secs(1), client.read_exact(&mut frame));
  read.await.unwrap().unwrap();
  assert_eq!(frame, [0x81, 2, b'h', b'i']);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_socket_moved_to_another_task_keeps_the_connection() {
  let (mut client, _) = run(1 << 20, None, |mut ws| async move {
    crate::ws::runtime::spawn(async move {
      if let Some(Ok(message)) = ws.recv().await {
        ws.send(message).await.unwrap();
      }
    });
  })
  .await;
  client
    .write_all(&client_frame(TEXT, b"echo"))
    .await
    .unwrap();
  let mut echo = [0; 6];
  client.read_exact(&mut echo).await.unwrap();
  assert_eq!(echo, [0x81, 4, b'e', b'c', b'h', b'o']);
  assert_eq!(read_close(&mut client).await, 1000);
  assert_eof(&mut client).await;
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_send_only_handler_sees_a_dead_peer() {
  let (tx, rx) = tokio::sync::oneshot::channel();
  let (client, _) = run(1 << 20, None, |mut ws| async move {
    let mut sent = 0u32;
    while sent < 1_000 && ws.send(Message::text("tick")).await.is_ok() {
      sent += 1;
      crate::ws::testing::yield_now().await;
    }
    let _ = tx.send(sent);
  })
  .await;
  drop(client);
  let sent = rx.await.unwrap();
  assert!(sent < 3, "{sent}");
}

#[cfg_attr(not(feature = "compio"), tokio::test(start_paused = true))]
#[cfg_attr(feature = "compio", compio::test)]
async fn max_lifetime_ends_a_recv_parked_in_another_task() {
  let (tx, rx) = tokio::sync::oneshot::channel();
  let (_client, _) = run(1 << 20, Some(Duration::from_secs(1)), |mut ws| async move {
    crate::ws::runtime::spawn(async move {
      let _ = tx.send(ws.recv().await.is_none());
    });
  })
  .await;
  let ended = crate::ws::testing::timeout(Duration::from_secs(30), rx)
    .await
    .expect("the parked recv is woken")
    .unwrap();
  assert!(ended);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn sixteen_pipelined_echoes_leave_in_one_write() {
  let (mut client, counts) = run(1 << 20, None, |mut ws| async move {
    while let Some(Ok(message)) = ws.recv().await {
      if ws.send(message).await.is_err() {
        break;
      }
    }
  })
  .await;
  let batch: Vec<u8> = (0..16).flat_map(|_| client_frame(TEXT, b"hello")).collect();
  client.write_all(&batch).await.unwrap();
  let mut echoes = [0; 16 * 7];
  client.read_exact(&mut echoes).await.unwrap();
  assert_eq!(counts.writes(), 1);
}
