use std::time::Duration;

use bytes::Bytes;
use futures_util::FutureExt;
use tokio::io::AsyncReadExt;

use super::Config;
use super::Core;
use super::WebSocket;
use crate::ws::Message;
use crate::ws::runtime::lock;
use crate::ws::runtime::shared;
use crate::ws::testing::timeout;
use crate::ws::testing::upgraded_pair;

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn cancelling_a_backpressured_send_does_not_enqueue_its_message() {
  let (upgraded, mut client, _) = upgraded_pair(64).await;
  let config = Config {
    write_buffer_size: 16,
    ..Config::default()
  };
  let core = shared(Core::new(upgraded, config));
  let mut ws = WebSocket::new(core.clone());
  crate::ws::testing::spawn(async move {
    lock(&core).owner.enter();
    ws.send(Message::Binary(Bytes::from_static(&[1; 128])))
      .await
      .unwrap();
    assert!(ws.send(Message::text("cancelled")).now_or_never().is_none());

    let mut first = [0; 132];
    let (flushed, read) = timeout(Duration::from_secs(5), async {
      futures_util::join!(ws.flush(), client.read_exact(&mut first))
    })
    .await
    .expect("the queued frame still flushes after cancellation");
    flushed.unwrap();
    read.unwrap();
    assert_eq!(first[..4], [0x82, 126, 0, 128]);
    assert_eq!(first[4..], [1; 128]);

    ws.send(Message::text("next")).await.unwrap();
    ws.flush().await.unwrap();
    let mut next = [0; 6];
    client.read_exact(&mut next).await.unwrap();
    assert_eq!(next, [0x81, 4, b'n', b'e', b'x', b't']);
    assert!(client.read_u8().now_or_never().is_none());
  })
  .await
  .unwrap();
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_backpressured_send_resumes_without_losing_or_repeating_its_message() {
  let (upgraded, mut client, _) = upgraded_pair(64).await;
  let config = Config {
    write_buffer_size: 16,
    ..Config::default()
  };
  let core = shared(Core::new(upgraded, config));
  let mut ws = WebSocket::new(core.clone());
  crate::ws::testing::spawn(async move {
    lock(&core).owner.enter();
    ws.send(Message::Binary(Bytes::from_static(&[2; 128])))
      .await
      .unwrap();
    let writer = async {
      {
        let send = ws.send(Message::text("next"));
        futures_util::pin_mut!(send);
        assert!(send.as_mut().now_or_never().is_none());
        send.await.unwrap();
      }
      ws.flush().await.unwrap();
    };
    let mut output = [0; 138];
    let ((), read) = timeout(Duration::from_secs(5), async {
      futures_util::join!(writer, client.read_exact(&mut output))
    })
    .await
    .expect("the pending send resumes when the peer reads");
    read.unwrap();
    assert_eq!(output[..4], [0x82, 126, 0, 128]);
    assert_eq!(output[4..132], [2; 128]);
    assert_eq!(output[132..], [0x81, 4, b'n', b'e', b'x', b't']);
    assert!(lock(&core).write_buf.is_empty());
    assert!(client.read_u8().now_or_never().is_none());
  })
  .await
  .unwrap();
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn cancelling_a_write_through_send_preserves_the_accepted_frame() {
  let (upgraded, mut client, _) = upgraded_pair(64).await;
  let core = shared(Core::new(upgraded, Config::default()));
  let mut ws = WebSocket::new(core.clone());
  assert!(
    ws.send(Message::Binary(Bytes::from_static(&[3; 128])))
      .now_or_never()
      .is_none()
  );

  let mut output = [0; 138];
  let (sent, read) = timeout(Duration::from_secs(5), async {
    futures_util::join!(
      ws.send(Message::text("next")),
      client.read_exact(&mut output)
    )
  })
  .await
  .expect("a later send flushes the accepted frame after cancellation");
  sent.unwrap();
  read.unwrap();
  assert_eq!(output[..4], [0x82, 126, 0, 128]);
  assert_eq!(output[4..132], [3; 128]);
  assert_eq!(output[132..], [0x81, 4, b'n', b'e', b'x', b't']);
  assert!(lock(&core).write_buf.is_empty());
  assert!(client.read_u8().now_or_never().is_none());
}
