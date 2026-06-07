//! Cache-friendly map keyed by HTTP method, backing the router's route tables.

use http::Method;

/// Maps the 9 standard HTTP methods to array indices.
/// Returns `None` for non-standard / extension methods.
#[inline]
fn method_slot(method: &Method) -> Option<usize> {
  Some(match *method {
    Method::GET => 0,
    Method::POST => 1,
    Method::PUT => 2,
    Method::DELETE => 3,
    Method::PATCH => 4,
    Method::HEAD => 5,
    Method::OPTIONS => 6,
    Method::CONNECT => 7,
    Method::TRACE => 8,
    _ => return None,
  })
}

/// Reconstructs a `Method` from its slot index.
///
/// The only caller iterates `0..9` over the `standard` array, so out-of-range
/// indices indicate an internal invariant violation. In debug builds this
/// trips an assertion; in release we degrade to `Method::GET` rather than
/// panic from a hot router path.
#[inline]
fn method_from_slot(idx: usize) -> Method {
  match idx {
    0 => Method::GET,
    1 => Method::POST,
    2 => Method::PUT,
    3 => Method::DELETE,
    4 => Method::PATCH,
    5 => Method::HEAD,
    6 => Method::OPTIONS,
    7 => Method::CONNECT,
    8 => Method::TRACE,
    _ => {
      debug_assert!(false, "method_from_slot called with idx={idx}");
      Method::GET
    }
  }
}

/// A compact, cache-friendly map keyed by HTTP method.
///
/// Standard methods (GET, POST, PUT, …) use O(1) array indexing.
/// Non-standard methods fall back to linear scan (extremely rare in practice).
pub(crate) struct MethodMap<V> {
  standard: [Option<V>; 9],
  custom: Vec<(Method, V)>,
}

impl<V> MethodMap<V> {
  pub(crate) fn new() -> Self {
    Self {
      standard: std::array::from_fn(|_| None),
      custom: Vec::new(),
    }
  }

  /// O(1) lookup for standard methods, linear scan for custom.
  #[inline]
  pub(crate) fn get(&self, method: &Method) -> Option<&V> {
    if let Some(idx) = method_slot(method) {
      self.standard[idx].as_ref()
    } else {
      self
        .custom
        .iter()
        .find(|(m, _)| m == method)
        .map(|(_, v)| v)
    }
  }

  /// Returns a mutable reference, inserting `V::default()` if absent.
  pub(crate) fn get_or_default_mut(&mut self, method: &Method) -> &mut V
  where
    V: Default,
  {
    if let Some(idx) = method_slot(method) {
      self.standard[idx].get_or_insert_with(V::default)
    } else {
      let pos = self.custom.iter().position(|(m, _)| m == method);
      if let Some(pos) = pos {
        &mut self.custom[pos].1
      } else {
        self.custom.push((method.clone(), V::default()));
        // SAFETY-style invariant: we just pushed, so the vec is non-empty.
        // Using `expect` over `unwrap` to surface the invariant if it ever
        // breaks in a future refactor.
        &mut self
          .custom
          .last_mut()
          .expect("custom vec must contain the entry we just pushed")
          .1
      }
    }
  }

  /// Iterates over all `(Method, &V)` pairs (standard then custom).
  pub(crate) fn iter(&self) -> impl Iterator<Item = (Method, &V)> {
    self
      .standard
      .iter()
      .enumerate()
      .filter_map(|(idx, slot)| slot.as_ref().map(|v| (method_from_slot(idx), v)))
      .chain(self.custom.iter().map(|(m, v)| (m.clone(), v)))
  }

  /// Mutable counterpart of [`MethodMap::iter`]. Used by router GC paths.
  pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut V> {
    self
      .standard
      .iter_mut()
      .filter_map(|slot| slot.as_mut())
      .chain(self.custom.iter_mut().map(|(_, v)| v))
  }
}
