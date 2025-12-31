use anyhow::Result;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tako::signals::Signal;
use tako::signals::app_events;
use tako::signals::ids;
use tako::types::Request;
use tokio::net::TcpListener;

async fn hello(_: Request) -> impl Responder {
  "Hello from signals example".into_response()
}

fn init_signals() {
  let arbiter = app_events();

  // Simple callback-style handler for server start
  arbiter.on(ids::SERVER_STARTED, |signal: Signal| async move {
    println!("[signals-basic] server.started: {:?}", signal.metadata);
  });

  // Stream-style listener for completed requests
  let mut rx = arbiter.subscribe(ids::REQUEST_COMPLETED);
  tokio::spawn(async move {
    while let Ok(signal) = rx.recv().await {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let path = signal.metadata.get("path").cloned().unwrap_or_default();
      let status = signal.metadata.get("status").cloned().unwrap_or_default();

      println!(
        "[signals-basic] request.completed: {} {} -> {}",
        method, path, status
      );
    }
  });
}

#[tokio::main]
async fn main() -> Result<()> {
  // Initialize signal listeners before starting the server
  init_signals();

  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", hello);

  tako::serve(listener, router).await;

  Ok(())
}
