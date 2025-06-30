use anyhow::Result;
use http::{Method, StatusCode};
use tako::{
    extractors::{
        AsyncFromRequestMut,
        multipart::{InMemoryFile, TakoTypedMultipart, UploadedFile},
    },
    responder::Responder,
    router::Router,
    types::Request,
};
use tokio::net::TcpListener;

async fn upload_file(mut req: Request) -> impl Responder {
    #[derive(serde::Deserialize)]
    struct Form {
        data: String,
    }

    let TakoTypedMultipart::<Form, UploadedFile> { data, .. } =
        TakoTypedMultipart::from_request(&mut req).await.unwrap();
    println!("Received file: {}", data.data);

    (StatusCode::OK, "File uploaded successfully")
}

async fn upload_mem(mut req: Request) -> impl Responder {
    #[derive(serde::Deserialize)]
    struct ImgForm {
        title: String,
    }

    let TakoTypedMultipart::<ImgForm, InMemoryFile> { data, .. } =
        TakoTypedMultipart::from_request(&mut req).await.unwrap();
    println!("Received image: {}", data.title);

    (StatusCode::OK, "Image uploaded successfully")
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::POST, "/upload_file", upload_file);
    router.route(Method::POST, "/upload_mem", upload_mem);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
