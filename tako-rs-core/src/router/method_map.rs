//! Cache-friendly map keyed by HTTP method, backing the router's route tables.

use http::Method;

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

/// Inverse of `method_slot`; out-of-range indices are a bug and yield GET in release builds.
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

/// Standard methods index an array; extension methods fall back to a linear scan.
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
}
