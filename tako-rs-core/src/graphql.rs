//! Async-`GraphQL` integration for Tako: extractors, responses, and subscriptions.
//!
//! - GraphQLRequest / GraphQLBatchRequest extractors
//! - GraphQLResponse / GraphQLBatchResponse responders
//! - GraphQLSubscription responder for WebSocket subscriptions
//! - APQ (Apollo Persisted Queries) and execution-cost limits via submodules
//!
//! Enable via the `async-graphql` cargo feature.
//!
//! # DataLoader integration
//!
//! `async_graphql::dataloader::DataLoader` is the canonical way to batch
//! `Object` field resolvers in N+1 patterns. Tako does not wrap it — instead,
//! attach the loader to per-request `Data` and pull it from the resolver:
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
//! Field resolver:
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
#![cfg(feature = "async-graphql")]
#![cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]

/// Apollo Persisted Queries (APQ) flow.
pub mod apq;
/// Execution-cost limits (max depth, max complexity).
pub mod limits;

mod protocol;
mod request;
mod response;
#[cfg(not(feature = "compio"))]
mod subscription;
#[cfg(not(feature = "compio"))]
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
pub use request::set_global_graphql_options;
pub use response::GraphQLBatchResponse;
pub use response::GraphQLResponse;
#[cfg(not(feature = "compio"))]
pub use subscription::GraphQLSubscription;
#[cfg(not(feature = "compio"))]
pub use websocket::GraphQLWebSocket;

#[cfg(feature = "graphiql")]
pub use crate::graphiql::GraphiQL;
#[cfg(feature = "graphiql")]
pub use crate::graphiql::graphiql;
