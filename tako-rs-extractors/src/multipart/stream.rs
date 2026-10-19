use std::task::Context;
use std::task::Poll;

use super::MultipartConfig;
use super::MultipartError;

/// Streaming multipart fields with the configured size, part-count, and content-type limits.
/// Previously returned fields must be dropped before reading the next one.
pub struct MultipartFields<'a> {
  inner: multer::Multipart<'a>,
  config: MultipartConfig,
  count: usize,
  limit_exceeded: bool,
}

impl<'a> MultipartFields<'a> {
  pub(super) fn new(inner: multer::Multipart<'a>, config: MultipartConfig) -> Self {
    Self {
      inner,
      config,
      count: 0,
      limit_exceeded: false,
    }
  }

  pub async fn next_field(&mut self) -> Result<Option<multer::Field<'a>>, MultipartError> {
    std::future::poll_fn(|cx| self.poll_next_field(cx)).await
  }

  pub async fn next_field_with_idx(
    &mut self,
  ) -> Result<Option<(usize, multer::Field<'a>)>, MultipartError> {
    self
      .next_field()
      .await
      .map(|field| field.map(|field| (field.index(), field)))
  }

  pub fn poll_next_field(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Option<multer::Field<'a>>, MultipartError>> {
    if self.limit_exceeded {
      return Poll::Ready(Err(MultipartError::TooManyParts));
    }
    let field = match self.inner.poll_next_field(cx) {
      Poll::Pending => return Poll::Pending,
      Poll::Ready(Ok(Some(field))) => field,
      Poll::Ready(Ok(None)) => return Poll::Ready(Ok(None)),
      Poll::Ready(Err(error)) => return Poll::Ready(Err(MultipartError::FieldError(error))),
    };
    if self.config.max_parts.is_some_and(|max| self.count >= max) {
      self.limit_exceeded = true;
      return Poll::Ready(Err(MultipartError::TooManyParts));
    }
    self.count += 1;
    let content_type = field.content_type().map(std::string::ToString::to_string);
    if !self.config.content_type_ok(content_type.as_deref()) {
      return Poll::Ready(Err(MultipartError::DisallowedContentType(
        content_type.unwrap_or_default(),
      )));
    }
    Poll::Ready(Ok(Some(field)))
  }
}
