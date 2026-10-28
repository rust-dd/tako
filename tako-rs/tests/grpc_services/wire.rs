use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct HealthRequest {
  #[prost(string, tag = "1")]
  pub service: String,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct HealthResponse {
  #[prost(int32, tag = "1")]
  pub status: i32,
}

#[derive(Clone, Copy, PartialEq, Message)]
pub struct HealthListRequest {}

#[derive(Clone, PartialEq, Message)]
pub struct HealthListResponse {
  #[prost(btree_map = "string, message", tag = "1")]
  pub statuses: std::collections::BTreeMap<String, HealthResponse>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ReflectionRequest {
  #[prost(string, tag = "1")]
  pub host: String,
  #[prost(oneof = "Query", tags = "3, 4, 5, 6, 7")]
  pub query: Option<Query>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Query {
  #[prost(string, tag = "3")]
  Filename(String),
  #[prost(string, tag = "4")]
  Symbol(String),
  #[prost(message, tag = "5")]
  Extension(Extension),
  #[prost(string, tag = "6")]
  ExtensionNumbers(String),
  #[prost(string, tag = "7")]
  Services(String),
}

#[derive(Clone, PartialEq, Message)]
pub struct Extension {
  #[prost(string, tag = "1")]
  pub containing_type: String,
  #[prost(int32, tag = "2")]
  pub number: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ReflectionResponse {
  #[prost(string, tag = "1")]
  pub valid_host: String,
  #[prost(message, optional, tag = "2")]
  pub original_request: Option<ReflectionRequest>,
  #[prost(oneof = "Answer", tags = "4, 5, 6, 7")]
  pub answer: Option<Answer>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Answer {
  #[prost(message, tag = "4")]
  Files(Files),
  #[prost(message, tag = "5")]
  ExtensionNumbers(ExtensionNumbers),
  #[prost(message, tag = "6")]
  Services(Services),
  #[prost(message, tag = "7")]
  Error(Error),
}

#[derive(Clone, PartialEq, Message)]
pub struct Files {
  #[prost(bytes = "vec", repeated, tag = "1")]
  pub descriptors: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExtensionNumbers {
  #[prost(string, tag = "1")]
  pub base_type_name: String,
  #[prost(int32, repeated, tag = "2")]
  pub numbers: Vec<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Services {
  #[prost(message, repeated, tag = "1")]
  pub services: Vec<Service>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Service {
  #[prost(string, tag = "1")]
  pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Error {
  #[prost(int32, tag = "1")]
  pub code: i32,
  #[prost(string, tag = "2")]
  pub message: String,
}
