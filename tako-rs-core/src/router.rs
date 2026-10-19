//! The [`Router`], which matches requests to routes and runs them through middleware.
//!
//! # Examples
//!
//! ```rust
//! use tako::{router::Router, Method, responder::Responder, types::Request};
//!
//! async fn hello(_req: Request) -> impl Responder {
//!     "Hello, World!"
//! }
//!
//! async fn user_handler(_req: Request) -> impl Responder {
//!     "User profile"
//! }
//!
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! router.route(Method::GET, "/users/{id}", user_handler);
//!
//! // Add global middleware
//! router.middleware(|req, next| async move {
//!     println!("Processing request to: {}", req.uri());
//!     next.run(req).await
//! });
//! ```

mod definition;
mod dispatch;
mod fast_path;
mod introspection;
mod layers;
mod method_map;
mod mounting;
#[cfg(all(test, feature = "plugins"))]
mod plugin_tests;
mod plugins;
mod registration;
#[cfg(feature = "signals")]
mod request_signals;
mod state;
mod timeout;

pub use definition::Router;
pub(crate) use fast_path::Dispatch;
pub use layers::ErrorHandler;
pub use layers::ErrorHandlerWithParts;
pub use mounting::TAKO_ROUTES;
