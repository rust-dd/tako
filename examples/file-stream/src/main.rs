use anyhow::Result;
use tako::{Method, file_stream::FileStream, responder::Responder, router::Router, types::Request};
use tokio::{fs::File, net::TcpListener};
use tokio_util::io::ReaderStream;

async fn serve_file(_: Request) -> impl Responder {
  let file = File::open("test.txt").await.unwrap();
  let stream = ReaderStream::new(file);
  let file_stream = FileStream::new(stream, Some("test.txt".to_string()), None);
  file_stream.into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/file", serve_file);

  tako::serve(listener, router).await;
  Ok(())
}
