//! In-process named signals for cross-cutting hooks such as metrics, logging and app events.

mod arbiter;
mod arbiter_rpc;
mod rpc;
mod runtime;
mod signal;

pub mod transport;

pub use arbiter::SignalArbiter;
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
pub use signal::ids;
