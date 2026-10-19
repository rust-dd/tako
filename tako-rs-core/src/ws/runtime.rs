//! The only runtime-specific parts of the WebSocket engine.

#[cfg(feature = "compio")]
pub(crate) use compio_runtime::Owner;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::Shared;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::Sleep;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::lock;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::shared;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::sleep;
#[cfg(feature = "compio")]
pub(crate) use compio_runtime::spawn;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::Owner;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::Shared;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::Sleep;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::lock;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::shared;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::sleep;
#[cfg(not(feature = "compio"))]
pub(crate) use tokio_runtime::spawn;

#[cfg(not(feature = "compio"))]
mod tokio_runtime {
  use std::future::Future;
  use std::pin::Pin;
  use std::sync::Arc;
  use std::sync::Mutex;
  use std::sync::MutexGuard;
  use std::sync::PoisonError;
  use std::time::Duration;

  /// State shared by a connection's `WebSocket` handles and its driver.
  pub(crate) type Shared<T> = Arc<Mutex<T>>;

  pub(crate) fn shared<T>(value: T) -> Shared<T> {
    Arc::new(Mutex::new(value))
  }

  pub(crate) fn lock<T>(shared: &Shared<T>) -> MutexGuard<'_, T> {
    // A handler panic is caught by the driver; the core stays usable for the close frame.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
  }

  /// The driver's task while it polls the handler: sends from there are queued until it yields.
  #[derive(Default)]
  pub(crate) struct Owner {
    task: Option<tokio::task::Id>,
    polling: bool,
  }

  impl Owner {
    pub(crate) fn enter(&mut self) {
      if self.task.is_none() {
        self.task = tokio::task::try_id();
      }
      self.polling = true;
    }

    pub(crate) fn leave(&mut self) {
      self.polling = false;
    }

    pub(crate) fn is_current(&self) -> bool {
      self.polling && self.task.is_some() && tokio::task::try_id() == self.task
    }
  }

  pub(crate) fn spawn(future: impl Future<Output = ()> + Send + 'static) {
    tokio::spawn(future);
  }

  pub(crate) type Sleep = Pin<Box<tokio::time::Sleep>>;

  pub(crate) fn sleep(duration: Duration) -> Sleep {
    Box::pin(tokio::time::sleep(duration))
  }
}

#[cfg(feature = "compio")]
mod compio_runtime {
  use std::cell::RefCell;
  use std::cell::RefMut;
  use std::future::Future;
  use std::pin::Pin;
  use std::rc::Rc;
  use std::time::Duration;

  /// State shared by a connection's `WebSocket` handles and its driver.
  pub(crate) type Shared<T> = Rc<RefCell<T>>;

  pub(crate) fn shared<T>(value: T) -> Shared<T> {
    Rc::new(RefCell::new(value))
  }

  pub(crate) fn lock<T>(shared: &Shared<T>) -> RefMut<'_, T> {
    shared.borrow_mut()
  }

  /// Set while the driver polls the handler; on one thread that alone identifies its sends.
  #[derive(Default)]
  pub(crate) struct Owner {
    polling: bool,
  }

  impl Owner {
    pub(crate) fn enter(&mut self) {
      self.polling = true;
    }

    pub(crate) fn leave(&mut self) {
      self.polling = false;
    }

    pub(crate) fn is_current(&self) -> bool {
      self.polling
    }
  }

  pub(crate) fn spawn(future: impl Future<Output = ()> + 'static) {
    compio::runtime::spawn(future).detach();
  }

  pub(crate) type Sleep = Pin<Box<dyn Future<Output = ()>>>;

  pub(crate) fn sleep(duration: Duration) -> Sleep {
    Box::pin(compio::time::sleep(duration))
  }
}
