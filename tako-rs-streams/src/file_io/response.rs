use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::types::Response;

use super::OpenFile;
use super::conditional::evaluate_conditional;
use super::date::format_http_date;
use super::etag::weak_etag_from_metadata;
use super::range::RequestedRange;
use super::range::requested_range;
use super::range::unsatisfiable;

pub(crate) async fn serve(
  file: OpenFile,
  original: &Path,
  encoding: Option<&'static str>,
  vary: bool,
  cache_control: Option<&HeaderValue>,
  request: &http::request::Parts,
) -> io::Result<Response> {
  let modified = file
    .modified
    .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
    .map(|ts| format_http_date(ts.as_secs()));
  let etag = file.modified.map(|modified| {
    let mut etag = weak_etag_from_metadata(file.size, modified);
    if let Some(encoding) = encoding {
      etag.pop();
      etag.push('-');
      etag.push_str(encoding);
      etag.push('"');
    }
    etag
  });
  let mut response = if let Some(response) =
    evaluate_conditional(&request.headers, etag.as_deref(), file.modified)
  {
    response
  } else {
    let range = if request.method == Method::GET {
      requested_range(&request.headers, file.size, etag.as_deref())
    } else {
      RequestedRange::Full
    };
    let (start, length, status) = match range {
      RequestedRange::Unsatisfiable => return Ok(unsatisfiable(file.size)),
      RequestedRange::Full => (0, file.size, StatusCode::OK),
      RequestedRange::Partial { start, length } => (start, length, StatusCode::PARTIAL_CONTENT),
    };
    let mut response = Response::new(TakoBody::empty());
    *response.status_mut() = status;
    response
      .headers_mut()
      .insert(header::CONTENT_LENGTH, length.into());
    response.headers_mut().insert(
      header::CONTENT_TYPE,
      mime_guess::from_path(original)
        .first_or_octet_stream()
        .as_ref()
        .parse()
        .unwrap(),
    );
    if status == StatusCode::PARTIAL_CONTENT {
      response.headers_mut().insert(
        header::CONTENT_RANGE,
        format!(
          "bytes {start}-{}/{total}",
          start + length - 1,
          total = file.size
        )
        .parse()
        .unwrap(),
      );
    }
    if request.method != Method::HEAD {
      *response.body_mut() = TakoBody::from_stream(file.into_stream(start, length).await?);
    }
    response
  };
  let headers = response.headers_mut();
  headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
  if let Some(etag) = etag {
    headers.insert(header::ETAG, etag.parse().unwrap());
  }
  if let Some(modified) = modified {
    headers.insert(header::LAST_MODIFIED, modified.parse().unwrap());
  }
  if let Some(encoding) = encoding {
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
  }
  if vary {
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
  }
  if let Some(cache_control) = cache_control {
    headers.insert(header::CACHE_CONTROL, cache_control.clone());
  }
  Ok(response)
}

pub(crate) fn method_error(request: &http::request::Parts) -> Option<Response> {
  if matches!(request.method, Method::GET | Method::HEAD) {
    return None;
  }
  Some(
    http::Response::builder()
      .status(StatusCode::METHOD_NOT_ALLOWED)
      .header(header::ALLOW, "GET, HEAD")
      .body(TakoBody::empty())
      .unwrap(),
  )
}
