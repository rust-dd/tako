//! Standard bidirectional server reflection, including descriptor dependencies and extensions.

use std::sync::Arc;

use descriptors::Descriptors;
use descriptors::File;
use descriptors::error;
use futures_util::StreamExt;
use parking_lot::RwLock;
use prost::Message;
use server_reflection_request::MessageRequest;
use server_reflection_response::MessageResponse;

use super::GrpcBidi;
use super::GrpcDeadline;
use super::GrpcStatus;
use super::GrpcStatusCode;
use crate::router::Router;

mod descriptors;
mod error;
mod protocol;
pub use error::DescriptorError;
pub use protocol::*;

/// Encoded v1 and v1alpha reflection schemas.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("proto/reflection.bin");

/// Services and file descriptors to expose; clones share one registry.
#[derive(Clone, Default)]
pub struct ReflectionRegistry {
  inner: Arc<RwLock<Descriptors>>,
}

impl ReflectionRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  /// Registers the v1 and v1alpha routes and their own descriptors.
  pub fn register(&self, router: &mut Router) {
    self
      .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
      .expect("valid built-in reflection descriptors");
    for path in [
      "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo",
      "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
    ] {
      let registry = self.clone();
      router.post(
        path,
        move |deadline: Option<GrpcDeadline>,
              bidi: GrpcBidi<ServerReflectionRequest, ServerReflectionResponse>| {
          let registry = registry.clone();
          async move {
            bidi
              .respond(move |requests| {
                requests.map(move |request| {
                  request
                    .map(|request| registry.respond(request))
                    .map_err(GrpcStatus::from)
                })
              })
              .with_deadline(deadline)
          }
        },
      );
    }
  }

  /// Imports a `FileDescriptorSet`, indexing services, symbols, and extensions automatically.
  /// Generate it with `protoc --include_imports`; missing imports produce `NotFound` replies.
  /// Invalid protobuf or unnamed files return an error without changing the registry.
  pub fn register_encoded_file_descriptor_set(&self, bytes: &[u8]) -> Result<(), DescriptorError> {
    let set = EncodedDescriptorSet::decode(bytes)?;
    let mut files = Vec::with_capacity(set.files.len());
    for bytes in set.files {
      let descriptor = prost_types::FileDescriptorProto::decode(bytes.as_slice())?;
      let name = descriptor.name().to_owned();
      if name.is_empty() {
        return Err(DescriptorError::MissingFileName);
      }
      files.push((
        name,
        File {
          bytes,
          descriptor: Some(descriptor),
        },
      ));
    }
    let mut inner = self.inner.write();
    inner.files.extend(files);
    inner.reindex();
    Ok(())
  }

  /// Registers a fully-qualified service name such as `helloworld.Greeter`.
  pub fn add_service(&self, name: impl Into<String>) {
    self.inner.write().services.insert(name.into());
  }

  /// Stores a serialized `FileDescriptorProto`, replacing the same filename and its index.
  /// Prefer `register_encoded_file_descriptor_set` to validate descriptors before serving.
  pub fn add_file(&self, filename: impl Into<String>, descriptor: Vec<u8>) {
    let mut inner = self.inner.write();
    inner.files.insert(filename.into(), File::new(descriptor));
    inner.reindex();
  }

  /// Maps a symbol to its file, overriding the automatically indexed mapping.
  pub fn map_symbol(&self, symbol: impl Into<String>, filename: impl Into<String>) {
    self
      .inner
      .write()
      .symbols
      .insert(symbol.into(), filename.into());
  }

  /// Sorted, deduplicated names from descriptors and `add_service`.
  pub fn list_services(&self) -> Vec<String> {
    self.inner.read().services()
  }

  pub fn file_by_filename(&self, filename: &str) -> Option<Vec<u8>> {
    self
      .inner
      .read()
      .files
      .get(filename)
      .map(|file| file.bytes.clone())
  }

  /// Resolves an indexed symbol or an explicit `map_symbol` mapping.
  pub fn file_containing_symbol(&self, symbol: &str) -> Option<Vec<u8>> {
    let inner = self.inner.read();
    let filename = inner.filename_for_symbol(symbol)?;
    inner.files.get(filename).map(|file| file.bytes.clone())
  }

  fn respond(&self, request: ServerReflectionRequest) -> ServerReflectionResponse {
    let answer = self
      .answer(request.message_request.as_ref())
      .unwrap_or_else(MessageResponse::ErrorResponse);
    ServerReflectionResponse {
      valid_host: request.host.clone(),
      original_request: Some(request),
      message_response: Some(answer),
    }
  }

  fn answer(&self, request: Option<&MessageRequest>) -> Result<MessageResponse, ErrorResponse> {
    let inner = self.inner.read();
    let filename = match request {
      Some(MessageRequest::ListServices(_)) => {
        return Ok(MessageResponse::ListServicesResponse(ListServiceResponse {
          service: inner
            .services()
            .into_iter()
            .map(|name| ServiceResponse { name })
            .collect(),
        }));
      }
      Some(MessageRequest::FileByFilename(filename)) => filename,
      Some(MessageRequest::FileContainingSymbol(symbol)) => {
        inner.filename_for_symbol(symbol).ok_or_else(|| {
          error(
            GrpcStatusCode::NotFound,
            format!("symbol not found: {symbol}"),
          )
        })?
      }
      Some(MessageRequest::FileContainingExtension(extension)) => inner
        .index
        .extensions
        .get(&extension.containing_type)
        .and_then(|extensions| extensions.get(&extension.extension_number))
        .ok_or_else(|| error(GrpcStatusCode::NotFound, "extension not found"))?,
      Some(MessageRequest::AllExtensionNumbersOfType(name)) => {
        if !inner.index.messages.contains(name) {
          return Err(error(
            GrpcStatusCode::NotFound,
            format!("message type not found: {name}"),
          ));
        }
        return Ok(MessageResponse::AllExtensionNumbersResponse(
          ExtensionNumberResponse {
            base_type_name: name.clone(),
            extension_number: inner
              .index
              .extensions
              .get(name)
              .map(|extensions| extensions.keys().copied().collect())
              .unwrap_or_default(),
          },
        ));
      }
      None => {
        return Err(error(
          GrpcStatusCode::InvalidArgument,
          "missing reflection query",
        ));
      }
    };
    Ok(MessageResponse::FileDescriptorResponse(
      FileDescriptorResponse {
        file_descriptor_proto: inner.with_dependencies(filename)?,
      },
    ))
  }
}

/// Router-state wrapper that lets plugin or middleware code find the active registry.
pub struct ReflectionState {
  pub registry: ReflectionRegistry,
}

// Retaining the embedded bytes preserves custom options that prost-types does not recognize.
#[derive(prost::Message)]
struct EncodedDescriptorSet {
  #[prost(bytes = "vec", repeated, tag = "1")]
  files: Vec<Vec<u8>>,
}
