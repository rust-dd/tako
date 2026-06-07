//! In-process signal arbiter and dispatch system.
//!
//! This module defines a small abstraction for named signals that can be emitted
//! and handled within a Tako application. It is intended for cross-cutting
//! concerns such as metrics, logging hooks, or custom application events.

mod arbiter;
mod arbiter_rpc;
mod rpc;
mod runtime;
mod signal;

/// Connection-lifecycle signal helpers used by every transport.
pub mod transport;

pub use arbiter::SignalArbiter;
pub use arbiter::app_events;
pub use arbiter::app_signals;
pub use rpc::RpcError;
pub use rpc::RpcResult;
pub use rpc::RpcTimeoutError;
pub use signal::FILTERED_SUBSCRIPTION_BUFFER;
pub use signal::MAX_BROADCAST_CAPACITY;
pub use signal::RpcHandler;
pub use signal::Signal;
pub use signal::SignalExporter;
pub use signal::SignalHandler;
pub use signal::SignalPayload;
pub use signal::SignalStream;
pub use signal::bus;
pub use signal::ids;
