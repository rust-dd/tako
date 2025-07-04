use anyhow::Result;
use tako::{
    Method,
    responder::Responder,
    router::Router,
    state::{get_state, set_state},
    types::Request,
};
use tokio::net::TcpListener;

async fn hello_world(_: Request) -> impl Responder {
    let names = get_state::<Vec<&str>>("names").unwrap();
    let age = get_state::<u32>("age").unwrap();
    let city = get_state::<&str>("city").unwrap();

    format!(
        "Hello , World! Names: {:?}, Age: {}, City: {}",
        names, age, city
    )
    .into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    set_state("names", vec!["Alice", "Bob", "Charlie"]);
    set_state("age", 25 as u32);
    set_state("city", "New York");

    let mut router = Router::new();
    router.route(Method::GET, "/", hello_world);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
