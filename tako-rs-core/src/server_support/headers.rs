use http::HeaderMap;
use http::header::CONNECTION;
use http::header::HeaderName;
use http_body::Frame;
use http_body_util::BodyExt;

use crate::body::TakoBody;
use crate::types::Response;

/// Removes hop-by-hop fields from an HTTP/2 or HTTP/3 response and its trailers.
pub fn sanitize_response(mut response: Response) -> Response {
  let nominated = strip_connection_headers(response.headers_mut());
  if !response.body().is_buffered() {
    let body = std::mem::take(response.body_mut());
    *response.body_mut() =
      TakoBody::new(body.map_frame(move |frame| match frame.into_trailers() {
        Ok(mut trailers) => {
          strip_connection_headers(&mut trailers);
          for name in &nominated {
            trailers.remove(name);
          }
          Frame::trailers(trailers)
        }
        Err(frame) => frame,
      }));
  }
  response
}

fn strip_connection_headers(headers: &mut HeaderMap) -> Vec<HeaderName> {
  let nominated: Vec<_> = headers
    .get_all(CONNECTION)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
    .collect();
  for name in &nominated {
    headers.remove(name);
  }
  for name in [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "te",
  ] {
    headers.remove(name);
  }
  nominated
}
