//! Wire messages for <https://github.com/grpc/grpc-proto/blob/master/grpc/health/v1/health.proto>.

/// Service to check; the empty name selects the whole server.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthCheckRequest {
  #[prost(string, tag = "1")]
  pub service: String,
}

#[derive(Clone, Copy, PartialEq, prost::Message)]
pub struct HealthCheckResponse {
  #[prost(enumeration = "health_check_response::ServingStatus", tag = "1")]
  pub status: i32,
}

pub mod health_check_response {
  /// The wire default is `Unknown`; the registry's convenience enum defaults to `Serving`.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
  #[repr(i32)]
  pub enum ServingStatus {
    Unknown = 0,
    Serving = 1,
    NotServing = 2,
    ServiceUnknown = 3,
  }
}

#[derive(Clone, Copy, PartialEq, prost::Message)]
pub struct HealthListRequest {}

/// Snapshot of explicitly registered services.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthListResponse {
  #[prost(btree_map = "string, message", tag = "1")]
  pub statuses: std::collections::BTreeMap<String, HealthCheckResponse>,
}
