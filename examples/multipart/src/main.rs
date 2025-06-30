async fn upload_file(mut req: Request) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Form {
        description: String,
        file: UploadedFile,
    }

    let TakoTypedMultipart::<Form, UploadedFile> { data, .. } =
        TakoTypedMultipart::from_request(&mut req).await?;

    Ok(format!(
        "Saved {} ({} bytes) at {:?}",
        data.file.file_name.unwrap_or("<unnamed>".into()),
        data.file.size,
        data.file.path
    ))
}

async fn upload_mem(mut req: Request) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct ImgForm {
        title: String,
        image: InMemoryFile,
    }

    let TakoTypedMultipart::<ImgForm, InMemoryFile> { data, .. } =
        TakoTypedMultipart::from_request(&mut req).await?;

    Ok(format!(
        "Received {}: {} bytes in RAM",
        data.image.file_name.unwrap_or("img".into()),
        data.image.data.len()
    ))
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
