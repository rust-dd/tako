use tako::extractors::typed_params::TypedParams;
use tako::responder::Responder;
use tako::route;
use tako::router::Router;

#[route(GET, "/users/{id: u64}")]
async fn get_user(TypedParams(p): TypedParams<GetUserParams>) -> impl Responder {
  format!("user id={}\n", p.id)
}

#[route(GET, "/users/{user_id: u64}/posts/{post_id: u64}")]
async fn get_post(TypedParams(p): TypedParams<GetPostParams>) -> impl Responder {
  format!("user={} post={}\n", p.user_id, p.post_id)
}

#[route(GET, "/items/{id: u64}/{name: String}/{ratio: f64}")]
async fn get_item(TypedParams(p): TypedParams<GetItemParams>) -> impl Responder {
  format!("id={} name={} ratio={}\n", p.id, p.name, p.ratio)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let mut router = Router::new();
  router.route(GetUserParams::METHOD, GetUserParams::PATH, get_user);
  router.route(GetPostParams::METHOD, GetPostParams::PATH, get_post);
  router.route(GetItemParams::METHOD, GetItemParams::PATH, get_item);

  let listener = tako::bind_with_port_fallback("127.0.0.1:3000").await?;
  println!("listening on {}", listener.local_addr()?);
  tako::serve(listener, router).await;
  Ok(())
}
