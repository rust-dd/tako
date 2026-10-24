#![cfg(unix)]

use std::fs::OpenOptions;
use std::future::Future;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use tako_rs_core::router::Router;
#[cfg(feature = "compio")]
use tako_rs_server::CompioServer as RuntimeServer;
#[cfg(not(feature = "compio"))]
use tako_rs_server::Server as RuntimeServer;

const CHILD_TIMEOUT: Duration = Duration::from_secs(10);

struct TestDirectory(PathBuf);

impl TestDirectory {
  fn new() -> Self {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
      "tako-us-{}-{}",
      std::process::id(),
      NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }

  fn socket(&self) -> PathBuf {
    self.0.join("socket")
  }
}

impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

struct ChildGuard(Child);

impl ChildGuard {
  fn spawn(test: &str, variable: &str, directory: &Path, name: &str) -> Self {
    Self(
      Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(variable, directory)
        .env("TAKO_TEST_UNIX_CHILD_NAME", name)
        .spawn()
        .unwrap(),
    )
  }

  fn finish(&mut self) {
    let deadline = Instant::now() + CHILD_TIMEOUT;
    loop {
      if let Some(status) = self.0.try_wait().unwrap() {
        assert!(status.success(), "child exited with {status}");
        return;
      }
      assert!(Instant::now() < deadline, "child did not finish");
      std::thread::sleep(Duration::from_millis(5));
    }
  }
}

impl Drop for ChildGuard {
  fn drop(&mut self) {
    let _ = self.0.kill();
    let _ = self.0.wait();
  }
}

fn wait_for(path: &Path) {
  let deadline = Instant::now() + CHILD_TIMEOUT;
  while !path.exists() {
    assert!(
      Instant::now() < deadline,
      "{} was not created",
      path.display()
    );
    std::thread::sleep(Duration::from_millis(5));
  }
}

fn run<T>(future: impl Future<Output = T>) -> T {
  #[cfg(not(feature = "compio"))]
  {
    tokio::runtime::Runtime::new().unwrap().block_on(future)
  }
  #[cfg(feature = "compio")]
  {
    compio::runtime::Runtime::new().unwrap().block_on(future)
  }
}

async fn wait_for_shutdown(path: &Path) {
  let deadline = Instant::now() + CHILD_TIMEOUT;
  while !path.exists() {
    assert!(
      Instant::now() < deadline,
      "parent did not stop the listener"
    );
    #[cfg(not(feature = "compio"))]
    tokio::time::sleep(Duration::from_millis(5)).await;
    #[cfg(feature = "compio")]
    compio::time::sleep(Duration::from_millis(5)).await;
  }
}

fn inode(path: &Path) -> u64 {
  std::fs::symlink_metadata(path).unwrap().ino()
}

fn error_kind(error: &tako_rs_core::types::BoxError) -> std::io::ErrorKind {
  error.downcast_ref::<std::io::Error>().unwrap().kind()
}

fn open_lock(directory: &Path) -> std::fs::File {
  OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(directory.join("socket.lock"))
    .unwrap()
}

#[test]
fn lock_contention_preserves_stale_socket() {
  if let Some(directory) = std::env::var_os("TAKO_TEST_UNIX_LOCK_DIR") {
    let directory = PathBuf::from(directory);
    let lock = open_lock(&directory);
    lock.try_lock().unwrap();
    std::fs::write(directory.join("ready"), []).unwrap();
    wait_for(&directory.join("release"));
    return;
  }

  let directory = TestDirectory::new();
  let socket = directory.socket();
  drop(UnixListener::bind(&socket).unwrap());
  let stale_inode = inode(&socket);
  let mut child = ChildGuard::spawn(
    "lock_contention_preserves_stale_socket",
    "TAKO_TEST_UNIX_LOCK_DIR",
    &directory.0,
    "lock",
  );
  wait_for(&directory.0.join("ready"));
  let lock_inode = inode(&directory.0.join("socket.lock"));
  let (error, retained_inode) = run(async {
    let result = RuntimeServer::builder()
      .build()
      .try_spawn_unix_http(&socket, Router::new())
      .await;
    let retained_inode = inode(&socket);
    let error = match result {
      Ok(server) => {
        server.shutdown(Duration::from_secs(1)).await;
        None
      }
      Err(error) => Some(error_kind(&error)),
    };
    (error, retained_inode)
  });
  std::fs::write(directory.0.join("release"), []).unwrap();
  child.finish();
  assert_eq!(error, Some(std::io::ErrorKind::AddrInUse));
  assert_eq!(retained_inode, stale_inode);

  run(async {
    let server = RuntimeServer::builder()
      .build()
      .try_spawn_unix_http(&socket, Router::new())
      .await
      .expect("the startup lock must be released when its process exits");
    server.shutdown(Duration::from_secs(1)).await;
  });
  assert!(!socket.exists());
  assert_eq!(inode(&directory.0.join("socket.lock")), lock_inode);
  open_lock(&directory.0).try_lock().unwrap();
}

#[test]
fn concurrent_starts_preserve_winning_listener() {
  if let Some(directory) = std::env::var_os("TAKO_TEST_UNIX_START_DIR") {
    let directory = PathBuf::from(directory);
    let name = std::env::var("TAKO_TEST_UNIX_CHILD_NAME").unwrap();
    run(async {
      std::fs::write(directory.join(format!("{name}.ready")), []).unwrap();
      wait_for(&directory.join("start"));
      let mut router = Router::new();
      router.get("/", || async { "winner" });
      let socket = directory.join("socket");
      match RuntimeServer::builder()
        .build()
        .try_spawn_unix_http(&socket, router)
        .await
      {
        Ok(server) => {
          let pending = directory.join(format!("{name}.pending"));
          std::fs::write(&pending, format!("ok:{}", inode(&socket))).unwrap();
          std::fs::rename(pending, directory.join(format!("{name}.result"))).unwrap();
          wait_for_shutdown(&directory.join("stop")).await;
          server.shutdown(Duration::from_secs(1)).await;
        }
        Err(error) => {
          assert_eq!(error_kind(&error), std::io::ErrorKind::AddrInUse);
          let pending = directory.join(format!("{name}.pending"));
          std::fs::write(&pending, "in-use").unwrap();
          std::fs::rename(pending, directory.join(format!("{name}.result"))).unwrap();
        }
      }
    });
    return;
  }

  let directory = TestDirectory::new();
  let socket = directory.socket();
  drop(UnixListener::bind(&socket).unwrap());
  let mut first = ChildGuard::spawn(
    "concurrent_starts_preserve_winning_listener",
    "TAKO_TEST_UNIX_START_DIR",
    &directory.0,
    "first",
  );
  let mut second = ChildGuard::spawn(
    "concurrent_starts_preserve_winning_listener",
    "TAKO_TEST_UNIX_START_DIR",
    &directory.0,
    "second",
  );
  wait_for(&directory.0.join("first.ready"));
  wait_for(&directory.0.join("second.ready"));
  std::fs::write(directory.0.join("start"), []).unwrap();
  wait_for(&directory.0.join("first.result"));
  wait_for(&directory.0.join("second.result"));
  let results = ["first", "second"]
    .map(|name| std::fs::read_to_string(directory.0.join(format!("{name}.result"))).unwrap());
  let winners: Vec<_> = results
    .iter()
    .filter_map(|s| s.strip_prefix("ok:"))
    .collect();
  assert_eq!(
    winners.len(),
    1,
    "both processes bound the same path: {results:?}"
  );
  let winning_inode = winners[0].parse::<u64>().unwrap();
  assert_eq!(inode(&socket), winning_inode);

  let error = run(async {
    RuntimeServer::builder()
      .build()
      .try_spawn_unix_http(&socket, Router::new())
      .await
      .expect_err("a subsequent startup must preserve the winning listener")
  });
  assert_eq!(error_kind(&error), std::io::ErrorKind::AddrInUse);
  assert_eq!(inode(&socket), winning_inode);
  open_lock(&directory.0).try_lock().unwrap();

  let mut client = UnixStream::connect(&socket).unwrap();
  client
    .set_read_timeout(Some(Duration::from_secs(2)))
    .unwrap();
  client
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .unwrap();
  let mut response = String::new();
  client.read_to_string(&mut response).unwrap();
  assert!(response.starts_with("HTTP/1.1 200"), "{response}");
  assert!(response.ends_with("winner"), "{response}");
  assert_eq!(inode(&socket), winning_inode);
  std::fs::write(directory.0.join("stop"), []).unwrap();
  first.finish();
  second.finish();
  assert!(directory.0.join("socket.lock").exists());
}
