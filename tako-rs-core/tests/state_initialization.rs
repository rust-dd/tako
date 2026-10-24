use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use tako_rs_core::state::get_or_init_state;
use tako_rs_core::state::get_state;

#[test]
fn nested_state_initialization_finishes() {
  run_bounded("nested_state_initialization_finishes", || {
    struct Slot<const N: usize>;
    get_or_init_state(|| Slot::<0>);
    macro_rules! initialize {
      ($($n:literal),*) => { $(
        get_or_init_state(|| {
          assert!(get_state::<Slot<0>>().is_some());
          get_or_init_state(|| Slot::<{ 1000 + $n }>);
          Slot::<$n>
        });
      )* };
    }
    initialize!(
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
      26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
      49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64
    );
  });
}

#[test]
fn recursive_initialization_panics_and_leaves_slot_retryable() {
  run_bounded(
    "recursive_initialization_panics_and_leaves_slot_retryable",
    || {
      struct RecursiveState;
      assert!(
        std::panic::catch_unwind(|| {
          get_or_init_state(|| {
            get_or_init_state(|| RecursiveState);
            RecursiveState
          });
        })
        .is_err()
      );
      assert!(get_state::<RecursiveState>().is_none());
      get_or_init_state(|| RecursiveState);
    },
  );
}

fn run_bounded(name: &str, test: impl FnOnce()) {
  if cfg!(miri) || std::env::var("TAKO_STATE_TEST_CHILD").as_deref() == Ok(name) {
    test();
    return;
  }

  let mut child = Command::new(std::env::current_exe().unwrap())
    .args(["--exact", name, "--nocapture"])
    .env("TAKO_STATE_TEST_CHILD", name)
    .stdout(Stdio::null())
    .spawn()
    .unwrap();
  let deadline = Instant::now() + Duration::from_secs(3);
  loop {
    if let Some(status) = child.try_wait().unwrap() {
      assert!(
        status.success(),
        "nested initialization child failed: {status}"
      );
      break;
    }
    if Instant::now() >= deadline {
      child.kill().unwrap();
      child.wait().unwrap();
      panic!("state initialization deadlocked in {name}");
    }
    std::thread::sleep(Duration::from_millis(10));
  }
}

#[test]
fn concurrent_initialization_runs_once() {
  struct ConcurrentState;
  let calls = AtomicUsize::new(0);
  let ready = Barrier::new(8);
  let values = std::thread::scope(|scope| {
    let handles: Vec<_> = (0..8)
      .map(|_| {
        scope.spawn(|| {
          ready.wait();
          get_or_init_state(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            ConcurrentState
          })
        })
      })
      .collect();
    handles
      .into_iter()
      .map(|handle| handle.join().unwrap())
      .collect::<Vec<_>>()
  });
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
}

#[test]
fn failed_initializer_can_be_retried() {
  struct RetriedState;
  assert!(
    std::panic::catch_unwind(|| {
      get_or_init_state::<RetriedState, _>(|| panic!("initializer failed"));
    })
    .is_err()
  );
  assert!(get_state::<RetriedState>().is_none());
  let state = get_or_init_state(|| RetriedState);
  assert!(Arc::ptr_eq(&state, &get_state::<RetriedState>().unwrap()));
}
