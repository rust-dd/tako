use tako::extractors::typed_params::TypedParams;
use tako::responder::Responder;
use tako::router::Router;
use tako::{delete, get, post};

#[get("/users/{id: u64}")]
async fn get_user(TypedParams(p): TypedParams<GetUserParams>) -> impl Responder {
  format!("user id={}\n", p.id)
}

#[get("/users/{user_id: u64}/posts/{post_id: u64}")]
async fn get_post(TypedParams(p): TypedParams<GetPostParams>) -> impl Responder {
  format!("user={} post={}\n", p.user_id, p.post_id)
}

#[get("/items/{id: u64}/{name: String}/{ratio: f64}")]
async fn get_item(TypedParams(p): TypedParams<GetItemParams>) -> impl Responder {
  format!("id={} name={} ratio={}\n", p.id, p.name, p.ratio)
}

#[post("/users/{id: u64}/posts")]
async fn create_post(TypedParams(p): TypedParams<CreatePostParams>) -> impl Responder {
  format!("created post for user={}\n", p.id)
}

#[delete("/users/{id: u64}")]
async fn delete_user(TypedParams(p): TypedParams<DeleteUserParams>) -> impl Responder {
  format!("deleted user={}\n", p.id)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let mut router = Router::new();
  router.mount_all();

  let listener = tako::bind_with_port_fallback("127.0.0.1:3000").await?;
  println!("listening on {}", listener.local_addr()?);
  tako::serve(listener, router).await;
  Ok(())
}
