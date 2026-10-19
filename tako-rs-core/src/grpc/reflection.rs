//! Descriptor registry for `grpc.reflection.v1`: answers `list_services`, `file_by_filename`
//! and `file_containing_symbol`, while the RPC itself is up to you.

use std::sync::Arc;

use parking_lot::RwLock;
use scc::HashMap as SccHashMap;

/// Services and file descriptors to expose; clones share one registry.
#[derive(Clone, Default)]
pub struct ReflectionRegistry {
  services: Arc<RwLock<Vec<String>>>,
  files: Arc<SccHashMap<String, Vec<u8>>>,
  symbols: Arc<SccHashMap<String, String>>,
}

impl ReflectionRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  /// Registers a fully-qualified service name such as `helloworld.Greeter`.
  pub fn add_service(&self, name: impl Into<String>) {
    self.services.write().push(name.into());
  }

  /// Registers a serialized `FileDescriptorProto`, replacing one under the same filename.
  pub fn add_file(&self, filename: impl Into<String>, descriptor: Vec<u8>) {
    self.files.upsert_sync(filename.into(), descriptor);
  }

  /// Maps a symbol such as `pkg.Service.Method` to its file, replacing an earlier mapping.
  pub fn map_symbol(&self, symbol: impl Into<String>, filename: impl Into<String>) {
    self.symbols.upsert_sync(symbol.into(), filename.into());
  }

  pub fn list_services(&self) -> Vec<String> {
    self.services.read().clone()
  }

  pub fn file_by_filename(&self, filename: &str) -> Option<Vec<u8>> {
    self.files.get_sync(filename).map(|e| e.get().clone())
  }

  /// Resolves the file a symbol was mapped to with [`map_symbol`](Self::map_symbol).
  pub fn file_containing_symbol(&self, symbol: &str) -> Option<Vec<u8>> {
    let entry = self.symbols.get_sync(symbol)?;
    let filename = entry.get().clone();
    drop(entry);
    self.file_by_filename(&filename)
  }
}

/// Router-state wrapper that lets plugin or middleware code find the active registry.
pub struct ReflectionState {
  pub registry: ReflectionRegistry,
}
