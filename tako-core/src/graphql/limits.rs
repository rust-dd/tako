//! GraphQL execution-cost limits.
//!
//! Thin re-export layer over `async_graphql::extensions::Logger` /
//! `MaxDepth` / `MaxComplexity`-style controls, exposing one `Limits` builder
//! that callers attach to their schema before serving requests.
//!
//! ```rust,ignore
//! use async_graphql::Schema;
//! use tako::graphql::limits::Limits;
//!
//! let schema = Schema::build(Q, M, S)
//!     .limit_depth(10)
//!     .limit_complexity(1_000)
//!     .finish();
//!
//! // Equivalently:
//! let limits = Limits::new().max_depth(10).max_complexity(1_000);
//! let schema = limits.apply(Schema::build(Q, M, S)).finish();
//! ```

#[cfg(feature = "async-graphql")]
use async_graphql::SchemaBuilder;

/// Builder for execution-cost limits.
#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
  pub max_depth: Option<usize>,
  pub max_complexity: Option<usize>,
}

impl Limits {
  /// Empty limits (no caps).
  pub fn new() -> Self {
    Self::default()
  }

  /// Cap query depth — depth is the longest selection-set chain.
  pub fn max_depth(mut self, n: usize) -> Self {
    self.max_depth = Some(n);
    self
  }

  /// Cap query complexity — sum of `@cost`-annotated field weights.
  pub fn max_complexity(mut self, n: usize) -> Self {
    self.max_complexity = Some(n);
    self
  }

  /// Apply both limits to an `async_graphql::SchemaBuilder`.
  #[cfg(feature = "async-graphql")]
  pub fn apply<Q, M, S>(self, mut b: SchemaBuilder<Q, M, S>) -> SchemaBuilder<Q, M, S> {
    if let Some(d) = self.max_depth {
      b = b.limit_depth(d);
    }
    if let Some(c) = self.max_complexity {
      b = b.limit_complexity(c);
    }
    b
  }
}
