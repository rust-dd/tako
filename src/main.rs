use hyper::Method;
use serde::Deserialize;
use tako::{
    body::TakoBody,
    extractors::{FromRequest, bytes::Bytes, header_map::HeaderMap, params::Params},
    responder::Responder,
    state::get_state,
    types::{Request, Response},
};

#[derive(Clone, Default)]
pub struct AppState {
    pub count: u32,
}

pub async fn hello(mut req: Request) -> impl Responder {
    let HeaderMap(headers) = HeaderMap::from_request(&mut req).await.unwrap();
    let Bytes(bytes) = Bytes::from_request(&mut req).await.unwrap();

    "Hello, World!".into_response()
}

#[derive(Deserialize, Debug)]
pub struct Par {
    pub id: u32,
}

pub async fn user_created(mut req: Request) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    let Params(params) = Params::<Par>::from_request(&mut req).await.unwrap();
    println!("User ID: {:?}", params);

    String::from("User created").into_response()
}

#[derive(Deserialize, Debug)]
pub struct UserCompanyParams {
    pub user_id: u32,
    pub company_id: u32,
}

pub async fn user_company(mut req: Request) -> impl Responder {
    let state = get_state::<AppState>("app_state").unwrap();
    let Params(params) = Params::<UserCompanyParams>::from_request(&mut req)
        .await
        .unwrap();
    println!("User ID: {:?}", params);

    String::from("User created").into_response()
}

pub async fn middleware(req: Request) -> Result<Request, Response> {
    // Your middleware logic here
    if false {
        return Err(hyper::Response::builder()
            .status(401)
            .body(TakoBody::empty())
            .unwrap()
            .into_response());
    }
    Ok(req)
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let mut r = tako::router::Router::new();
    r.state("app_state", AppState::default());

    r.route(Method::GET, "/", hello)
        .middleware(middleware)
        .middleware(middleware);
    r.route_with_tsr(Method::POST, "/user", user_created);
    r.route_with_tsr(Method::POST, "/user/{id}", user_created);
    r.route_with_tsr(
        Method::POST,
        "/user/{user_id}/company/{company_id}",
        user_company,
    );
    tako::serve(listener, r).await;
}
