use std::time::Duration;

use futures_util::future::Either;
use tokio::sync::oneshot;

use super::UploadWriter;
use super::blocking;

struct NotifyOnDrop(Option<oneshot::Sender<()>>);

impl Drop for NotifyOnDrop {
  fn drop(&mut self) {
    let _ = self.0.take().unwrap().send(());
  }
}

async fn cancelled_creation_drops_its_result() {
  let (started, path_ready) = oneshot::channel();
  let (release, proceed) = std::sync::mpsc::channel();
  let (finished, dropped) = oneshot::channel();
  let mut creation = Box::pin(blocking(move || {
    let writer = UploadWriter::create_blocking()?;
    started.send(writer.path.clone()).unwrap();
    proceed.recv().unwrap();
    Ok((writer, NotifyOnDrop(Some(finished))))
  }));
  let path = match futures_util::future::select(path_ready, &mut creation).await {
    Either::Left((path, _)) => path.unwrap(),
    Either::Right(_) => panic!("the blocking operation must still be waiting"),
  };
  drop(creation);
  assert!(path.exists());
  release.send(()).unwrap();
  dropped.await.unwrap();
  assert!(
    !path.exists(),
    "a cancelled operation must drop its completed upload result"
  );
}

#[tokio::test]
async fn cancelled_background_creation_removes_file() {
  tokio::time::timeout(
    Duration::from_secs(2),
    cancelled_creation_drops_its_result(),
  )
  .await
  .unwrap();
}

#[cfg(feature = "compio")]
#[compio::test]
async fn compio_cancelled_background_creation_removes_file() {
  compio::time::timeout(
    Duration::from_secs(2),
    cancelled_creation_drops_its_result(),
  )
  .await
  .unwrap();
}
