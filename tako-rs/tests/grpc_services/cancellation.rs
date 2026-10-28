use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::Next;
use tako::router::Router;
use tako::types::Request;

struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
  fn drop(&mut self) {
    self.0.fetch_add(1, Ordering::SeqCst);
  }
}

pub fn monitor(router: &Router, dropped: &Arc<AtomicUsize>) {
  let dropped = dropped.clone();
  router.middleware(move |request: Request, next: Next| {
    let watch = request.uri().path() == "/grpc.health.v1.Health/Watch";
    let dropped = dropped.clone();
    async move {
      let mut response = next.run(request).await;
      if watch {
        let body = std::mem::take(response.body_mut());
        let stream = futures_util::stream::unfold((body, DropCounter(dropped)), |(mut body, counter)| async move {
          body.frame().await.map(|frame| (frame, (body, counter)))
        });
        *response.body_mut() = TakoBody::from_try_stream(stream);
      }
      response
    }
  });
}

pub async fn wait(dropped: &AtomicUsize, previous: usize) {
  tokio::time::timeout(Duration::from_secs(2), async {
    while dropped.load(Ordering::SeqCst) == previous {
      tokio::time::sleep(Duration::from_millis(5)).await;
    }
  })
  .await
  .expect("the server retained a cancelled health watch");
}
