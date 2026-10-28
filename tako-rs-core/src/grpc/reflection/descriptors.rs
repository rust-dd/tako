use std::collections::BTreeMap;
use std::collections::BTreeSet;

use prost::Message;
use prost_types::DescriptorProto;
use prost_types::EnumDescriptorProto;
use prost_types::FieldDescriptorProto;
use prost_types::FileDescriptorProto;

use super::ErrorResponse;
use crate::grpc::GrpcStatusCode;

pub(super) struct File {
  pub bytes: Vec<u8>,
  pub descriptor: Option<FileDescriptorProto>,
}

impl File {
  pub fn new(bytes: Vec<u8>) -> Self {
    let descriptor = FileDescriptorProto::decode(bytes.as_slice()).ok();
    Self { bytes, descriptor }
  }
}

#[derive(Default)]
pub(super) struct Descriptors {
  pub files: BTreeMap<String, File>,
  pub services: BTreeSet<String>,
  pub symbols: BTreeMap<String, String>,
  pub index: Index,
}

impl Descriptors {
  pub fn reindex(&mut self) {
    let mut index = Index::default();
    for (filename, file) in &self.files {
      if let Some(descriptor) = &file.descriptor {
        index.insert(filename, descriptor);
      }
    }
    self.index = index;
  }

  pub fn services(&self) -> Vec<String> {
    self.services.union(&self.index.services).cloned().collect()
  }

  pub fn filename_for_symbol(&self, symbol: &str) -> Option<&str> {
    self
      .symbols
      .get(symbol)
      .or_else(|| self.index.symbols.get(symbol))
      .map(String::as_str)
  }

  pub fn with_dependencies(&self, filename: &str) -> Result<Vec<Vec<u8>>, ErrorResponse> {
    let mut pending = vec![filename];
    let mut visited = BTreeSet::new();
    let mut descriptors = Vec::new();
    while let Some(filename) = pending.pop() {
      if !visited.insert(filename) {
        continue;
      }
      let file = self.files.get(filename).ok_or_else(|| {
        error(
          GrpcStatusCode::NotFound,
          format!("file not found: {filename}"),
        )
      })?;
      let descriptor = file.descriptor.as_ref().ok_or_else(|| {
        error(
          GrpcStatusCode::Internal,
          format!("invalid descriptor: {filename}"),
        )
      })?;
      descriptors.push(file.bytes.clone());
      pending.extend(descriptor.dependency.iter().rev().map(String::as_str));
    }
    Ok(descriptors)
  }
}

#[derive(Default)]
pub(super) struct Index {
  pub services: BTreeSet<String>,
  pub symbols: BTreeMap<String, String>,
  pub messages: BTreeSet<String>,
  pub extensions: BTreeMap<String, BTreeMap<i32, String>>,
}

impl Index {
  fn insert(&mut self, filename: &str, file: &FileDescriptorProto) {
    let package = file.package();
    for service in &file.service {
      let name = self.symbol(package, service.name(), filename);
      self.services.insert(name.clone());
      for method in &service.method {
        self.symbol(&name, method.name(), filename);
      }
    }
    for message in &file.message_type {
      self.message(package, message, filename);
    }
    for enumeration in &file.enum_type {
      self.enumeration(package, enumeration, filename);
    }
    for extension in &file.extension {
      self.extension(package, extension, filename);
    }
  }

  fn symbol(&mut self, prefix: &str, name: &str, filename: &str) -> String {
    let name = if prefix.is_empty() {
      name.to_owned()
    } else {
      format!("{prefix}.{name}")
    };
    self.symbols.insert(name.clone(), filename.to_owned());
    name
  }

  fn message(&mut self, prefix: &str, message: &DescriptorProto, filename: &str) {
    let name = self.symbol(prefix, message.name(), filename);
    self.messages.insert(name.clone());
    for field in &message.field {
      self.symbol(&name, field.name(), filename);
    }
    for oneof in &message.oneof_decl {
      self.symbol(&name, oneof.name(), filename);
    }
    for nested in &message.nested_type {
      self.message(&name, nested, filename);
    }
    for enumeration in &message.enum_type {
      self.enumeration(&name, enumeration, filename);
    }
    for extension in &message.extension {
      self.extension(&name, extension, filename);
    }
  }

  fn enumeration(&mut self, prefix: &str, enumeration: &EnumDescriptorProto, filename: &str) {
    self.symbol(prefix, enumeration.name(), filename);
    for value in &enumeration.value {
      // Protobuf enum values occupy the enclosing scope, unlike Rust variants.
      self.symbol(prefix, value.name(), filename);
    }
  }

  fn extension(&mut self, prefix: &str, extension: &FieldDescriptorProto, filename: &str) {
    self.symbol(prefix, extension.name(), filename);
    self
      .extensions
      .entry(extension.extendee().trim_start_matches('.').to_owned())
      .or_default()
      .insert(extension.number(), filename.to_owned());
  }
}

pub(super) fn error(code: GrpcStatusCode, message: impl Into<String>) -> ErrorResponse {
  ErrorResponse {
    error_code: code as i32,
    error_message: message.into(),
  }
}
