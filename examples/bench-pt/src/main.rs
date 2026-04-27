use std::env;
use std::sync::atomic::{AtomicU64, Ordering};

use tako::Method;
use tako::PerThreadConfig;
use tako::responder::Responder;
use tako::router::Router;

// Per-worker request counter so we can see if SO_REUSEPORT is actually load-balancing.
thread_local! {
  static WORKER_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

static GLOBAL_HITS: AtomicU64 = AtomicU64::new(0);

async fn hello() -> impl Responder {
  WORKER_HITS.with(|c| c.set(c.get() + 1));
  GLOBAL_HITS.fetch_add(1, Ordering::Relaxed);
  "Hello, World!"
}

fn build_router() -> Router {
  let mut router = Router::new();
  router.route(Method::GET, "/", hello);
  router
}

fn spawn_reporter() {
  // Print thread-name -> hits every 5s so we can see the distribution.
  std::thread::spawn(|| {
    let mut last = 0u64;
    loop {
      std::thread::sleep(std::time::Duration::from_secs(5));
      let total = GLOBAL_HITS.load(Ordering::Relaxed);
      let delta = total - last;
      last = total;
      eprintln!("[reporter] +{delta} req/5s, total={total}");
    }
  });
}

fn main() {
  let mode = env::args().nth(1).unwrap_or_else(|| "multi".to_string());
  let addr = env::args().nth(2).unwrap_or_else(|| "127.0.0.1:8080".to_string());
  let workers: usize = env::args()
    .nth(3)
    .and_then(|s| s.parse().ok())
    .unwrap_or_else(|| {
      std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
    });

  eprintln!("bench-pt mode={mode} addr={addr} workers={workers}");
  spawn_reporter();

  match mode.as_str() {
    "multi" => {
      let rt = tokio::runtime::Runtime::new().expect("tokio rt");
      rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
        tako::serve(listener, build_router()).await;
      });
    }
    "pt-tokio" => {
      let cfg = PerThreadConfig {
        workers,
        pin_to_core: false,
        backlog: 1024,
      };
      tako::serve_per_thread(&addr, build_router(), cfg).expect("serve_per_thread");
    }
    "pt-compio" => {
      let cfg = PerThreadConfig {
        workers,
        pin_to_core: false,
        backlog: 1024,
      };
      tako::serve_per_thread_compio(&addr, build_router(), cfg)
        .expect("serve_per_thread_compio");
    }
    _ => {
      eprintln!("unknown mode '{mode}', expected: multi | pt-tokio | pt-compio");
      std::process::exit(2);
    }
  }
}
