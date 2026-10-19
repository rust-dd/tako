use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
#[cfg(feature = "compio")]
use std::time::Instant;

#[cfg(not(feature = "compio"))]
use tokio::time::Instant;

#[cfg(not(feature = "compio"))]
type Sleep = Pin<Box<tokio::time::Sleep>>;
#[cfg(feature = "compio")]
type Sleep = send_wrapper::SendWrapper<Pin<Box<dyn Future<Output = ()>>>>;

/// Arms on first poll because Compio timers belong to the thread driving the response.
pub(super) struct KeepAliveTimer {
  deadline: Instant,
  sleep: Option<Sleep>,
}

impl KeepAliveTimer {
  pub(super) fn new(period: Duration) -> Self {
    Self {
      deadline: Instant::now() + period,
      sleep: None,
    }
  }

  pub(super) fn reset(&mut self, period: Duration) {
    self.deadline = Instant::now() + period;
    #[cfg(not(feature = "compio"))]
    if let Some(sleep) = self.sleep.as_mut() {
      sleep.as_mut().reset(self.deadline);
    }
    #[cfg(feature = "compio")]
    {
      self.sleep = None;
    }
  }

  pub(super) fn poll_elapsed(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    let deadline = self.deadline;
    #[cfg(not(feature = "compio"))]
    let sleep = self
      .sleep
      .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
    #[cfg(feature = "compio")]
    let sleep = self.sleep.get_or_insert_with(|| {
      send_wrapper::SendWrapper::<Pin<Box<dyn Future<Output = ()>>>>::new(Box::pin(
        compio::time::sleep_until(deadline),
      ))
    });
    sleep.as_mut().poll(cx)
  }
}
