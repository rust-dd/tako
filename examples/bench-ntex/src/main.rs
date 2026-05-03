use std::env;
use std::sync::atomic::{AtomicU64, Ordering};

use ntex::web::{self, App, HttpServer};

static GLOBAL_HITS: AtomicU64 = AtomicU64::new(0);

async fn hello() -> &'static str {
  GLOBAL_HITS.fetch_add(1, Ordering::Relaxed);
  "Hello, World!"
}

fn spawn_reporter() {
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

#[ntex::main]
async fn main() -> std::io::Result<()> {
  let mode = env::args().nth(1).unwrap_or_else(|| "default".to_string());
  let addr = env::args().nth(2).unwrap_or_else(|| "127.0.0.1:8080".to_string());
  let workers: usize = env::args()
    .nth(3)
    .and_then(|s| s.parse().ok())
    .unwrap_or_else(|| {
      std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
    });

  eprintln!("bench-ntex mode={mode} addr={addr} workers={workers}");
  spawn_reporter();

  let server = HttpServer::new(async || {
    App::new().service(web::resource("/").route(web::get().to(hello)))
  })
  .bind(&addr)?;

  match mode.as_str() {
    "default" => server.workers(workers).run().await,
    "pt" => server.workers(workers).enable_affinity().run().await,
    _ => {
      eprintln!("unknown mode '{mode}', expected: default | pt");
      std::process::exit(2);
    }
  }
}
