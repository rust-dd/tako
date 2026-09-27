use bytes::Bytes;
use bytes::BytesMut;
use futures_util::StreamExt;
use http::StatusCode;
use http::header;
use http_body::Body;
use http_body_util::BodyExt;
use http_body_util::BodyStream;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::BoxError;
use tako_rs_core::types::Response;

use crate::stores::IdempotencyEntry;
use crate::stores::StoreResult;

pub(crate) fn conflict(inflight: bool) -> Response {
  let mut response = http::Response::builder().status(StatusCode::CONFLICT);
  if inflight {
    response = response.header(header::RETRY_AFTER, 3);
  }
  response.body(TakoBody::empty()).unwrap()
}

pub(crate) fn replay(entry: IdempotencyEntry) -> StoreResult<Response> {
  let mut response = Response::new(TakoBody::from(entry.body.clone()));
  *response.status_mut() = StatusCode::from_u16(entry.status)?;
  for (name, value) in entry.headers {
    response.headers_mut().append(
      http::HeaderName::from_bytes(name.as_bytes())?,
      http::HeaderValue::from_bytes(&value)?,
    );
  }
  response
    .headers_mut()
    .insert(header::CONTENT_LENGTH, entry.body.len().into());
  Ok(response)
}

pub(crate) fn filter_headers(headers: &http::HeaderMap) -> Vec<(String, Vec<u8>)> {
  const DENY: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "set-cookie",
  ];
  let connection = headers
    .get_all(header::CONNECTION)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .map(str::trim)
    .collect::<Vec<_>>();
  headers
    .iter()
    .filter(|(name, _)| {
      !DENY.contains(&name.as_str())
        && !connection
          .iter()
          .any(|field| name.as_str().eq_ignore_ascii_case(field))
    })
    .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
    .collect()
}

pub(crate) async fn cacheable_body(
  response: &mut Response,
  limit: usize,
) -> StoreResult<Option<Bytes>> {
  if response.status().is_informational()
    || response
      .body()
      .size_hint()
      .upper()
      .is_none_or(|size| size > limit as u64)
  {
    return Ok(None);
  }
  let mut body = std::mem::take(response.body_mut());
  let mut frames = Vec::new();
  let mut length = 0usize;
  while let Some(frame) = body.frame().await {
    let frame = frame?;
    length = length.saturating_add(frame.data_ref().map_or(0, Bytes::len));
    let cacheable = frame.is_data() && length <= limit;
    frames.push(frame);
    if !cacheable {
      *response.body_mut() = TakoBody::from_try_stream(
        futures_util::stream::iter(frames.into_iter().map(Ok::<_, BoxError>))
          .chain(BodyStream::new(body)),
      );
      return Ok(None);
    }
  }
  let mut bytes = BytesMut::with_capacity(length);
  for frame in frames {
    if let Ok(data) = frame.into_data() {
      bytes.extend_from_slice(&data);
    }
  }
  let bytes = bytes.freeze();
  *response.body_mut() = TakoBody::from(bytes.clone());
  Ok(Some(bytes))
}
