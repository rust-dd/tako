//! `GraphQL` HTTP request extraction: single and batch extractors, body-size
//! limits, content-type classification, and the `receive_*` helpers.

use async_graphql::BatchRequest as GqlBatchRequest;
use async_graphql::http::MultipartOptions;
use http::StatusCode;
use http_body_util::BodyExt;

use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Single `GraphQL` request extractor.
pub struct GraphQLRequest(pub async_graphql::Request);

impl GraphQLRequest {
  pub fn into_inner(self) -> async_graphql::Request {
    self.0
  }
}

/// Batch `GraphQL` request extractor.
pub struct GraphQLBatchRequest(pub GqlBatchRequest);

impl GraphQLBatchRequest {
  pub fn into_inner(self) -> GqlBatchRequest {
    self.0
  }
}

/// Cap on the raw POST body GraphQL extractors will buffer.
///
/// Async-graphql parses the entire request body into memory before it can
/// validate the query, so without an upstream limit a single unauthenticated
/// POST could buffer many GB and OOM the process. `4 MiB` matches the default
/// body limits of comparable frameworks (Apollo, Hasura, federation gateways)
/// and is large enough for any realistic GraphQL document plus variables.
pub const MAX_GRAPHQL_BODY_SIZE: usize = 4 * 1024 * 1024;

/// Errors that can occur while parsing `GraphQL` HTTP requests.
#[derive(Debug)]
pub enum GraphQLError {
  MissingQuery,
  BodyRead(String),
  /// Request body exceeds [`MAX_GRAPHQL_BODY_SIZE`] — either by
  /// advertised `Content-Length` or by actual streamed bytes.
  BodyTooLarge,
  InvalidJson(String),
  Parse(String),
  UnsupportedMediaType(String),
}

/// Per-request or global options for `GraphQL` extraction.
#[derive(Clone, Default)]
pub struct GraphQLOptions {
  pub multipart: MultipartOptions,
}

impl Responder for GraphQLError {
  fn into_response(self) -> Response {
    match self {
      GraphQLError::MissingQuery => {
        (StatusCode::BAD_REQUEST, "Missing GraphQL query").into_response()
      }
      GraphQLError::BodyRead(e) => {
        (StatusCode::BAD_REQUEST, format!("Failed to read body: {e}")).into_response()
      }
      GraphQLError::BodyTooLarge => (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("GraphQL body exceeds {MAX_GRAPHQL_BODY_SIZE} bytes"),
      )
        .into_response(),
      GraphQLError::InvalidJson(e) => {
        (StatusCode::BAD_REQUEST, format!("Invalid JSON: {e}")).into_response()
      }
      GraphQLError::Parse(e) => {
        (StatusCode::BAD_REQUEST, format!("Invalid request: {e}")).into_response()
      }
      GraphQLError::UnsupportedMediaType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("Unsupported GraphQL content-type: {ct}"),
      )
        .into_response(),
    }
  }
}

/// Returns the `GraphQL` POST body media-type bucket if the request's
/// `Content-Type` header advertises one async-graphql understands, or
/// `Err(UnsupportedMediaType)` otherwise. Used to fail fast before buffering
/// a body that the parser would reject anyway with a confusing message.
fn classify_graphql_content_type(ct: Option<&str>) -> Result<GraphQLBodyKind, GraphQLError> {
  let raw = ct.unwrap_or("").trim();
  if raw.is_empty() {
    return Err(GraphQLError::UnsupportedMediaType("<missing>".to_string()));
  }
  let essence = raw
    .split(';')
    .next()
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase();
  match essence.as_str() {
    "application/json" => Ok(GraphQLBodyKind::Json),
    "application/graphql" | "application/graphql-response+json" => Ok(GraphQLBodyKind::Graphql),
    "multipart/form-data" => Ok(GraphQLBodyKind::Multipart),
    _ => Err(GraphQLError::UnsupportedMediaType(raw.to_string())),
  }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum GraphQLBodyKind {
  Json,
  Graphql,
  Multipart,
}

#[inline]
fn resolve_opts(req: &Request) -> MultipartOptions {
  // Prefer per-request options in extensions
  if let Some(opts) = req.extensions().get::<GraphQLOptions>() {
    return opts.multipart;
  }
  // Fallback to global state
  if let Some(global) = crate::state::get_state::<GraphQLOptions>() {
    return global.as_ref().multipart;
  }
  MultipartOptions::default()
}

fn parse_get_request(req: &Request) -> Result<async_graphql::Request, GraphQLError> {
  let qs = req.uri().query().unwrap_or("");
  async_graphql::http::parse_query_string(qs).map_err(|e| GraphQLError::Parse(e.to_string()))
}

async fn read_body_bytes(req: &mut Request) -> Result<bytes::Bytes, GraphQLError> {
  // Pre-check the advertised length: if the client says >MAX up front,
  // refuse without touching the body at all. Defends against allocation
  // pressure from header-only flooders.
  if let Some(cl) = req.headers().get(http::header::CONTENT_LENGTH)
    && let Some(n) = cl.to_str().ok().and_then(|s| s.parse::<usize>().ok())
    && n > MAX_GRAPHQL_BODY_SIZE
  {
    return Err(GraphQLError::BodyTooLarge);
  }

  // Then wrap the body in `Limited` so a missing or lying Content-Length
  // (chunked transfer, HTTP/2 without length) still cannot drag us past
  // the cap. Same pattern as the idempotency / hmac / json-schema paths.
  let body = std::mem::take(req.body_mut());
  let limited = http_body_util::Limited::new(body, MAX_GRAPHQL_BODY_SIZE);
  match limited.collect().await {
    Ok(c) => Ok(c.to_bytes()),
    Err(e) => {
      // `Limited` surfaces `LengthLimitError` on cap overrun; otherwise
      // it's a transport / body error. Use the type-name to disambiguate
      // without depending on the private error path.
      if e
        .downcast_ref::<http_body_util::LengthLimitError>()
        .is_some()
      {
        Err(GraphQLError::BodyTooLarge)
      } else {
        Err(GraphQLError::BodyRead(e.to_string()))
      }
    }
  }
}

impl<'a> FromRequest<'a> for GraphQLRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.method() == http::Method::GET {
        return Ok(GraphQLRequest(parse_get_request(req)?));
      }

      // Resolve MultipartOptions: request extensions -> global state -> default
      let opts = resolve_opts(req);

      let content_type = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string);
      classify_graphql_content_type(content_type.as_deref())?;

      let body = read_body_bytes(req).await?;
      if body.is_empty() {
        return Err(GraphQLError::Parse("empty request body".to_string()));
      }

      let reader = futures_util::io::Cursor::new(body.to_vec());
      let req = async_graphql::http::receive_body(content_type.as_deref(), reader, opts)
        .await
        .map_err(|e| GraphQLError::Parse(e.to_string()))?;
      Ok(GraphQLRequest(req))
    }
  }
}

/// Helper to receive a single `GraphQL` request with custom `MultipartOptions`.
/// Attach per-request `GraphQL` options into request extensions.
pub fn attach_graphql_options(req: &mut Request, opts: GraphQLOptions) {
  req.extensions_mut().insert(opts);
}

/// Set global `GraphQL` options via Tako's global state.
pub fn set_global_graphql_options(opts: GraphQLOptions) {
  crate::state::set_state::<GraphQLOptions>(opts);
}

pub async fn receive_graphql(
  req: &mut Request,
  opts: MultipartOptions,
) -> Result<async_graphql::Request, GraphQLError> {
  if req.method() == http::Method::GET {
    return parse_get_request(req);
  }
  let body = read_body_bytes(req).await?;
  let content_type = req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(std::string::ToString::to_string);
  let reader = futures_util::io::Cursor::new(body.to_vec());
  async_graphql::http::receive_body(content_type.as_deref(), reader, opts)
    .await
    .map_err(|e| GraphQLError::Parse(e.to_string()))
}

/// Helper to receive a batch `GraphQL` request with custom `MultipartOptions`.
pub async fn receive_graphql_batch(
  req: &mut Request,
  opts: MultipartOptions,
) -> Result<GqlBatchRequest, GraphQLError> {
  if req.method() == http::Method::GET {
    let single = parse_get_request(req)?;
    return Ok(GqlBatchRequest::Single(single));
  }
  let content_type = req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(std::string::ToString::to_string);
  classify_graphql_content_type(content_type.as_deref())?;
  let body = read_body_bytes(req).await?;
  if body.is_empty() {
    return Err(GraphQLError::Parse("empty request body".to_string()));
  }
  let reader = futures_util::io::Cursor::new(body.to_vec());
  async_graphql::http::receive_batch_body(content_type.as_deref(), reader, opts)
    .await
    .map_err(|e| GraphQLError::Parse(e.to_string()))
}

impl<'a> FromRequest<'a> for GraphQLBatchRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.method() == http::Method::GET {
        // Treat GET as single request
        let single = parse_get_request(req)?;
        return Ok(GraphQLBatchRequest(GqlBatchRequest::Single(single)));
      }

      // Resolve MultipartOptions: request extensions -> global state -> default
      let opts = resolve_opts(req);

      let content_type = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string);
      classify_graphql_content_type(content_type.as_deref())?;
      let body = read_body_bytes(req).await?;
      if body.is_empty() {
        return Err(GraphQLError::Parse("empty request body".to_string()));
      }
      let reader = futures_util::io::Cursor::new(body.to_vec());
      let batch = async_graphql::http::receive_batch_body(content_type.as_deref(), reader, opts)
        .await
        .map_err(|e| GraphQLError::Parse(e.to_string()))?;
      Ok(GraphQLBatchRequest(batch))
    }
  }
}
