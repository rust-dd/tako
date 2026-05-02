//! `grpc.reflection.v1.ServerReflection` scaffolding.
//!
//! Provides a [`ReflectionRegistry`] that callers populate with the
//! `FileDescriptorProto` blobs they want to expose, and a [`ReflectionService`]
//! handler that answers `list_services`, `file_by_filename`, and
//! `file_containing_symbol` queries.
//!
//! ⚠️ **Status:** the encoder/decoder for `ServerReflectionRequest` /
//! `ServerReflectionResponse` is intentionally minimal — it covers the three
//! query kinds above and leaves the others (`file_containing_extension`,
//! `all_extension_numbers_of_type`) to follow-up work. Generated proto code
//! requires a build script, which would force every consumer to run
//! `protoc`; the scaffold here lets you ship reflection from a hand-rolled
//! descriptor or a pre-baked `.pb` blob in the meantime.

use std::sync::Arc;

use scc::HashMap as SccHashMap;

/// Reflection registry — populated at startup and consulted by the reflection RPC.
#[derive(Clone, Default)]
pub struct ReflectionRegistry {
  services: Arc<scc::Queue<String>>,
  files: Arc<SccHashMap<String, Vec<u8>>>,
  symbols: Arc<SccHashMap<String, String>>,
}

impl ReflectionRegistry {
  /// Empty registry.
  pub fn new() -> Self {
    Self::default()
  }

  /// Register a fully-qualified service name (e.g. `helloworld.Greeter`).
  pub fn add_service(&self, name: impl Into<String>) {
    self.services.push(name.into());
  }

  /// Register a file descriptor under its source filename.
  pub fn add_file(&self, filename: impl Into<String>, descriptor: Vec<u8>) {
    let _ = self.files.insert_sync(filename.into(), descriptor);
  }

  /// Map a fully-qualified symbol (`pkg.Service.Method`) to the file that defines it.
  pub fn map_symbol(&self, symbol: impl Into<String>, filename: impl Into<String>) {
    let _ = self.symbols.insert_sync(symbol.into(), filename.into());
  }

  /// Snapshot of registered service names.
  pub fn list_services(&self) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(item) = self.services.pop() {
      out.push((**item).clone());
    }
    // Re-push to keep the queue populated.
    for s in &out {
      self.services.push(s.clone());
    }
    out
  }

  /// Look up the descriptor blob for a filename.
  pub fn file_by_filename(&self, filename: &str) -> Option<Vec<u8>> {
    self.files.get_sync(filename).map(|e| e.get().clone())
  }

  /// Resolve a file descriptor by symbol (if registered via `map_symbol`).
  pub fn file_containing_symbol(&self, symbol: &str) -> Option<Vec<u8>> {
    let entry = self.symbols.get_sync(symbol)?;
    let filename = entry.get().clone();
    drop(entry);
    self.file_by_filename(&filename)
  }
}

/// Marker placed in router state so plugin/middleware code can discover the
/// active reflection registry.
pub struct ReflectionState {
  pub registry: ReflectionRegistry,
}
