//! Typed-RPC error model surfaced by the arbiter's `call_rpc*` methods.

/// Error type for typed RPC calls.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcError {
  /// No handler registered for the requested RPC method.
  NoHandler,
  /// The response type did not match the expected type.
  TypeMismatch,
}

impl std::fmt::Display for RpcError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::NoHandler => write!(f, "no handler registered for RPC method"),
      Self::TypeMismatch => write!(f, "RPC response type mismatch"),
    }
  }
}

impl std::error::Error for RpcError {}

/// Result type for RPC calls with explicit error reporting.
pub type RpcResult<T> = Result<T, RpcError>;

/// Error type for RPC calls with timeout support.
///
/// This error type implements `std::error::Error` for integration with
/// error handling libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcTimeoutError {
  /// The RPC call timed out before completing.
  Timeout,
  /// An RPC error occurred.
  Rpc(RpcError),
}

impl std::fmt::Display for RpcTimeoutError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Timeout => write!(f, "RPC call timed out"),
      Self::Rpc(err) => write!(f, "{err}"),
    }
  }
}

impl std::error::Error for RpcTimeoutError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Rpc(err) => Some(err),
      Self::Timeout => None,
    }
  }
}

impl From<RpcError> for RpcTimeoutError {
  #[inline]
  fn from(err: RpcError) -> Self {
    Self::Rpc(err)
  }
}
