use anyhow::Result;
use tako::middleware::upload_progress::{ProgressTracker, UploadProgress};
use tako::middleware::IntoMiddleware;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;
use tako::Method;

async fn upload_handler(req: Request) -> impl Responder {
  // Access the progress tracker from request extensions
  let info = req
    .extensions()
    .get::<ProgressTracker>()
    .map(|tracker| {
      let bytes = tracker.bytes_read();
      let total = tracker.total_bytes();
      let pct = tracker.percent().unwrap_or(0);
      format!("Upload complete: {bytes} bytes received (total: {total:?}, {pct}%)")
    })
    .unwrap_or_else(|| "No progress tracker found".to_string());

  info
}

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  let progress = UploadProgress::new()
    .on_progress(|state| {
      let pct = state.percent().map(|p| format!("{p}%")).unwrap_or_else(|| "?%".into());
      println!(
        "Upload progress: {} / {} bytes ({pct})",
        state.bytes_read,
        state.total_bytes.map(|t| t.to_string()).unwrap_or_else(|| "unknown".into()),
      );
    })
    .min_notify_interval_bytes(1024); // Notify at most every 1KB

  let mw = progress.into_middleware();

  let mut router = Router::new();
  router
    .route(Method::POST, "/upload", upload_handler)
    .middleware(mw);

  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
  println!("Upload progress server on http://127.0.0.1:8080");
  println!("Test with: curl -X POST -d @somefile http://127.0.0.1:8080/upload");

  tako::serve(listener, router).await;

  Ok(())
}
