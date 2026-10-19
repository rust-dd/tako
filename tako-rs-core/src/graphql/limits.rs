//! Query depth and complexity caps applied to an `async_graphql` schema builder.
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

use async_graphql::SchemaBuilder;

/// Execution-cost caps for a schema; `None` leaves a cap off.
#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
  pub max_depth: Option<usize>,
  pub max_complexity: Option<usize>,
}

impl Limits {
  pub fn new() -> Self {
    Self::default()
  }

  /// Caps query depth, the longest chain of nested selection sets.
  pub fn max_depth(mut self, n: usize) -> Self {
    self.max_depth = Some(n);
    self
  }

  /// Caps query complexity, the sum of field complexities (1 each unless overridden).
  pub fn max_complexity(mut self, n: usize) -> Self {
    self.max_complexity = Some(n);
    self
  }

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
