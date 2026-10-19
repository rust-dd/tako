//! `async-graphql` extractors, responders and WebSocket subscriptions.
//!
//! # Examples
//!
//! Tako does not wrap `DataLoader`; attach one to each request's `Data` to batch resolvers:
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use async_graphql::{Context, dataloader::*};
//!
//! struct UserLoader(/* db handle */);
//!
//! impl Loader<u64> for UserLoader {
//!     type Value = User;
//!     type Error = Arc<dyn std::error::Error + Send + Sync>;
//!     async fn load(&self, keys: &[u64]) -> Result<HashMap<u64, User>, Self::Error> {
//!         // SELECT * FROM users WHERE id IN ($keys)
//!         # unimplemented!()
//!     }
//! }
//!
//! // Per-request: attach the loader to Data.
//! let loader = DataLoader::new(UserLoader(db_handle), tokio::spawn);
//! let request = async_graphql::Request::new(query).data(loader);
//! schema.execute(request).await
//! ```
//!
//! ```rust,ignore
//! use async_graphql::Object;
//!
//! struct Query;
//!
//! #[Object]
//! impl Query {
//!     async fn user(&self, ctx: &Context<'_>, id: u64) -> Option<User> {
//!         ctx.data_unchecked::<DataLoader<UserLoader>>()
//!             .load_one(id)
//!             .await
//!             .ok()
//!             .flatten()
//!     }
//! }
//! ```

pub mod apq;
pub mod limits;

mod protocol;
mod request;
mod response;
#[cfg(all(feature = "ws", not(feature = "compio")))]
mod subscription;
#[cfg(all(feature = "ws", not(feature = "compio")))]
mod websocket;

pub use protocol::GraphQLProtocol;
pub use protocol::GraphQLProtocolRejection;
pub use request::GraphQLBatchRequest;
pub use request::GraphQLError;
pub use request::GraphQLOptions;
pub use request::GraphQLRequest;
pub use request::MAX_GRAPHQL_BODY_SIZE;
pub use request::attach_graphql_options;
pub use request::receive_graphql;
pub use request::receive_graphql_batch;
pub use response::GraphQLBatchResponse;
pub use response::GraphQLResponse;
#[cfg(all(feature = "ws", not(feature = "compio")))]
pub use subscription::GraphQLSubscription;
#[cfg(all(feature = "ws", not(feature = "compio")))]
pub use websocket::GraphQLWebSocket;

#[cfg(feature = "graphiql")]
pub use crate::graphiql::GraphiQL;
#[cfg(feature = "graphiql")]
pub use crate::graphiql::graphiql;
