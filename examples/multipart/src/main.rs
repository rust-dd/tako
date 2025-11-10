use anyhow::Result;
use http::{Method, StatusCode};
use tako::{
  extractors::{
    FromRequest,
    multipart::{InMemoryFile, TakoMultipart, TakoTypedMultipart, UploadedFile},
  },
  responder::Responder,
  router::Router,
  types::Request,
};
use tokio::net::TcpListener;

async fn upload_file(mut req: Request) -> impl Responder {
  #[derive(serde::Deserialize)]
  struct Form {
    description: String,
    file: UploadedFile,
  }

  let TakoTypedMultipart::<Form, UploadedFile> { data, .. } =
    TakoTypedMultipart::from_request(&mut req).await.unwrap();

  (StatusCode::OK, "File uploaded successfully")
}

async fn upload_mem(mut req: Request) -> impl Responder {
  #[derive(serde::Deserialize)]
  struct ImgForm {
    title: String,
    image: InMemoryFile,
  }

  let TakoTypedMultipart::<ImgForm, InMemoryFile> { data, .. } =
    TakoTypedMultipart::from_request(&mut req).await.unwrap();

  (StatusCode::OK, "Image uploaded successfully")
}

async fn raw_with_file(mut req: Request) -> impl Responder {
  let TakoMultipart(mut mp) = TakoMultipart::from_request(&mut req).await.unwrap();

  let mut total_files = 0usize;
  while let Some(mut field) = mp.next_field().await.unwrap() {
    if field.file_name().is_some() {
      let fname = field
        .file_name()
        .map(|s| s.to_owned())
        .unwrap_or_else(|| "<unnamed>".into());

      total_files += 1;
      let mut size = 0usize;
      while let Some(chunk) = field.chunk().await.unwrap() {
        size += chunk.len();
      }
    }
  }

  (StatusCode::OK, format!("processed {} file(s)", total_files))
}

async fn raw_text(mut req: Request) -> impl Responder {
  use std::collections::HashMap;

  let TakoMultipart(mut mp) = TakoMultipart::from_request(&mut req).await.unwrap();
  let mut map = HashMap::new();

  while let Some(field) = mp.next_field().await.unwrap() {
    if field.file_name().is_some() {
      return (StatusCode::BAD_REQUEST, "file not accepted");
    }
    let name = field.name().unwrap_or("noname").to_owned();
    let text = field.text().await.unwrap();
    map.insert(name, text);
  }
  (StatusCode::OK, "text form processed")
}

async fn typed_text(mut req: Request) -> impl Responder {
  #[derive(serde::Deserialize)]
  struct LoginForm {
    username: String,
    password: String,
  }

  let TakoTypedMultipart::<LoginForm, UploadedFile> { data, .. } =
    TakoTypedMultipart::from_request(&mut req).await.unwrap();

  (StatusCode::OK, "typed text processed")
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::POST, "/upload_file", upload_file);
  router.route(Method::POST, "/upload_mem", upload_mem);
  router.route(Method::POST, "/raw_with_file", raw_with_file);
  router.route(Method::POST, "/raw_text", raw_text);
  router.route(Method::POST, "/typed_text", typed_text);

  tako::serve(listener, router).await;

  Ok(())
}
