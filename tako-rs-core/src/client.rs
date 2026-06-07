//! HTTP client implementations for making outbound requests with TLS support.
//!
//! This module provides HTTP clients for making requests to external services. It includes
//! `TakoClient` for plain HTTP connections and `TakoTlsClient` for secure HTTPS connections
//! using rustls. Both clients support HTTP/1.1 protocol and handle connection management
//! automatically. The clients are generic over body types to support different request
//! payload formats while maintaining type safety and performance.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::client::{TakoClient, TakoTlsClient};
//! use http_body_util::Empty;
//! use bytes::Bytes;
//! use http::Request;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Plain HTTP client
//! let mut client = TakoClient::<Empty<Bytes>>::new("httpbin.org", Some(80)).await?;
//! let request = Request::builder()
//!     .uri("/get")
//!     .body(Empty::new())?;
//! let response = client.request(request).await?;
//!
//! // HTTPS client with TLS
//! let mut tls_client = TakoTlsClient::<Empty<Bytes>>::new("httpbin.org", None).await?;
//! let tls_request = Request::builder()
//!     .uri("/get")
//!     .body(Empty::new())?;
//! let tls_response = tls_client.request(tls_request).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(docsrs, doc(cfg(feature = "client")))]

mod plain;
mod pooled;
mod tls;
mod trust_store;

pub use plain::TakoClient;
pub use pooled::V2Client;
pub use pooled::V2ClientBuilder;
pub use tls::TakoTlsClient;
