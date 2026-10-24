use std::sync::Arc;

use async_graphql::BatchRequest as GqlBatchRequest;
use async_graphql::http::MultipartOptions;
use http::StatusCode;
use http_body_util::BodyExt;

use crate::extractors::FromRequest;
use crate::extractors::body::BodyLimit;
use crate::extractors::body::BodyReadError;
use crate::responder::Responder;
use crate::router_state::RouterState;
use crate::types::Request;
use crate::types::Response;

/// Extracts one `GraphQL` request from a GET query string or a POST body.
pub struct GraphQLRequest(pub async_graphql::Request);

impl GraphQLRequest {
  pub fn into_inner(self) -> async_graphql::Request {
    self.0
  }
}

/// Extracts a batch of `GraphQL` requests; a GET query string yields a batch of one.
pub struct GraphQLBatchRequest(pub GqlBatchRequest);

impl GraphQLBatchRequest {
  pub fn into_inner(self) -> GqlBatchRequest {
    self.0
  }
}

/// Protocol ceiling for POST bodies; the request's [`BodyLimit`] may impose a smaller cap.
pub const MAX_GRAPHQL_BODY_SIZE: usize = 4 * 1024 * 1024;

/// Rejection of a `GraphQL` HTTP request, answered with a 400, 413 or 415.
#[derive(Debug)]
pub enum GraphQLError {
  BodyRead(String),
  /// The declared or streamed body exceeds [`BodyLimit`] or [`MAX_GRAPHQL_BODY_SIZE`].
  BodyTooLarge,
  Parse(String),
  UnsupportedMediaType(String),
}

/// Per-request options for the `GraphQL` extractors, set with [`attach_graphql_options`].
#[derive(Clone, Default)]
pub struct GraphQLOptions {
  pub multipart: MultipartOptions,
}

impl Responder for GraphQLError {
  fn into_response(self) -> Response {
    match self {
      GraphQLError::BodyRead(e) => {
        (StatusCode::BAD_REQUEST, format!("Failed to read body: {e}")).into_response()
      }
      GraphQLError::BodyTooLarge => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "GraphQL body exceeds the configured limit",
      )
        .into_response(),
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

enum BodyFormat {
  Json,
  Raw,
  Multipart,
}

/// Rejects unsupported content before the body is buffered.
fn check_content_type(ct: Option<&str>) -> Result<BodyFormat, GraphQLError> {
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
    "application/json" | "application/graphql-response+json" => Ok(BodyFormat::Json),
    "application/graphql" => Ok(BodyFormat::Raw),
    "multipart/form-data" => Ok(BodyFormat::Multipart),
    _ => Err(GraphQLError::UnsupportedMediaType(raw.to_string())),
  }
}

/// The request's options, then the router's state, then the process-global store.
fn resolve_opts(req: &Request) -> MultipartOptions {
  let extensions = req.extensions();
  if let Some(opts) = extensions.get::<GraphQLOptions>() {
    return opts.multipart;
  }
  extensions
    .get::<Arc<RouterState>>()
    .and_then(|state| state.get::<GraphQLOptions>())
    .or_else(crate::state::get_state::<GraphQLOptions>)
    .map(|opts| opts.multipart)
    .unwrap_or_default()
}

fn parse_get_request(req: &Request) -> Result<async_graphql::Request, GraphQLError> {
  let qs = req.uri().query().unwrap_or("");
  async_graphql::http::parse_query_string(qs).map_err(|e| GraphQLError::Parse(e.to_string()))
}

async fn read_body_bytes(req: &mut Request) -> Result<bytes::Bytes, GraphQLError> {
  let limit = req
    .extensions()
    .get::<BodyLimit>()
    .copied()
    .unwrap_or_default()
    .0
    .map_or(MAX_GRAPHQL_BODY_SIZE, |limit| {
      limit.min(MAX_GRAPHQL_BODY_SIZE)
    });
  if let Some(cl) = req.headers().get(http::header::CONTENT_LENGTH)
    && let Some(n) = cl.to_str().ok().and_then(|s| s.parse::<usize>().ok())
    && n > limit
  {
    return Err(GraphQLError::BodyTooLarge);
  }

  // `Limited` also caps a chunked body or one whose `Content-Length` lies.
  let body = std::mem::take(req.body_mut());
  let limited = http_body_util::Limited::new(body, limit);
  match limited.collect().await {
    Ok(c) => Ok(c.to_bytes()),
    Err(e) => Err(match BodyReadError::from_boxed(e) {
      BodyReadError::TooLarge => GraphQLError::BodyTooLarge,
      BodyReadError::Read(error) => GraphQLError::BodyRead(error),
      error @ BodyReadError::InvalidUtf8 => GraphQLError::BodyRead(error.to_string()),
    }),
  }
}

impl<'a> FromRequest<'a> for GraphQLRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let opts = resolve_opts(req);
      receive_graphql(req, opts).await.map(GraphQLRequest)
    }
  }
}

/// Sets the options the `GraphQL` extractors use for this request, e.g. from a middleware.
pub fn attach_graphql_options(req: &mut Request, opts: GraphQLOptions) {
  req.extensions_mut().insert(opts);
}

/// Reads one `GraphQL` request from a GET query string or a POST body with `opts`.
pub async fn receive_graphql(
  req: &mut Request,
  opts: MultipartOptions,
) -> Result<async_graphql::Request, GraphQLError> {
  receive_graphql_batch(req, opts)
    .await?
    .into_single()
    .map_err(|e| GraphQLError::Parse(e.to_string()))
}

/// Reads a batch of `GraphQL` requests with `opts`; a GET query string yields a batch of one.
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
  let format = check_content_type(content_type.as_deref())?;
  let body = read_body_bytes(req).await?;
  if body.is_empty() {
    return Err(GraphQLError::Parse("empty request body".to_string()));
  }
  if matches!(format, BodyFormat::Json | BodyFormat::Raw) {
    content_type
      .as_deref()
      .unwrap_or_default()
      .parse::<mime::Mime>()
      .map_err(|error| {
        GraphQLError::Parse(async_graphql::ParseRequestError::from(error).to_string())
      })?;
  }
  match format {
    BodyFormat::Json => serde_json::from_slice(&body).map_err(|error| {
      GraphQLError::Parse(
        async_graphql::ParseRequestError::InvalidRequest(Box::new(error)).to_string(),
      )
    }),
    BodyFormat::Raw => String::from_utf8(body.to_vec())
      .map(|query| GqlBatchRequest::Single(async_graphql::Request::new(query)))
      .map_err(|error| GraphQLError::Parse(format!("GraphQL query must be UTF-8: {error}"))),
    BodyFormat::Multipart => {
      let reader = futures_util::io::Cursor::new(body);
      async_graphql::http::receive_batch_body(content_type.as_deref(), reader, opts)
        .await
        .map_err(|e| GraphQLError::Parse(e.to_string()))
    }
  }
}

impl<'a> FromRequest<'a> for GraphQLBatchRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let opts = resolve_opts(req);
      receive_graphql_batch(req, opts)
        .await
        .map(GraphQLBatchRequest)
    }
  }
}
