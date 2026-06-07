use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use multer::Multipart;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::types::Request;

use crate::multipart::FromMultipartField;
use crate::multipart::MultipartConfig;
use crate::multipart::MultipartError;
use crate::multipart::TypedMultipartError;

/// Wrapper around `multer::Multipart` to provide additional functionality.
///
/// This wrapper provides a unified interface for processing multipart form data
/// while maintaining compatibility with the underlying `multer` crate. It can be
/// used for manual processing of multipart fields when more control is needed
/// than the typed multipart extractor provides.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::extractors::multipart::TakoMultipart;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
///
/// async fn manual_multipart_handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
///     let TakoMultipart(mut multipart) = TakoMultipart::from_request(&mut req).await?;
///
///     while let Some(field) = multipart.next_field().await? {
///         if let Some(name) = field.name() {
///             println!("Field name: {}", name);
///             if let Some(filename) = field.file_name() {
///                 println!("File: {}", filename);
///             }
///         }
///     }
///
///     Ok(())
/// }
/// ```
#[doc(alias = "multipart")]
pub struct TakoMultipart<'a>(pub Multipart<'a>);

impl<'a> TakoMultipart<'a> {
  /// Consumes the wrapper and returns the inner `Multipart` instance.
  #[inline]
  pub fn into_inner(self) -> Multipart<'a> {
    self.0
  }
}

impl<'a> FromRequest<'a> for TakoMultipart<'a> {
  type Error = MultipartError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_multipart(req))
  }
}

impl<'a> TakoMultipart<'a> {
  fn extract_multipart(req: &'a mut Request) -> Result<TakoMultipart<'a>, MultipartError> {
    let content_type = req
      .headers()
      .get(CONTENT_TYPE)
      .ok_or(MultipartError::MissingContentType)?;

    let content_type_str = content_type
      .to_str()
      .map_err(|_| MultipartError::InvalidUtf8)?;

    let boundary = multer::parse_boundary(content_type_str)
      .map_err(|e| MultipartError::BoundaryParseError(e.to_string()))?;

    let cfg = MultipartConfig::lookup(req.extensions());
    let constraints = cfg.to_constraints();
    let body_stream = req.body_mut().into_data_stream();
    Ok(TakoMultipart(Multipart::with_constraints(
      body_stream,
      boundary,
      constraints,
    )))
  }
}

/// Represents a strongly-typed multipart request.
///
/// This struct allows deserialization of multipart form data into a strongly-typed
/// structure, combining both file and text fields. It provides automatic handling
/// of different field types and deserializes the entire form into a single data structure.
///
/// # Type Parameters
///
/// * `T` - The target type to deserialize form data into
/// * `F` - The type used for file fields (must implement `FromMultipartField`)
#[doc(alias = "typed_multipart")]
pub struct TakoTypedMultipart<'a, T, F> {
  /// Deserialized data from the multipart request.
  pub data: T,
  /// Marker for the field type (used for type inference).
  _marker: core::marker::PhantomData<&'a F>,
}

impl<'a, T, F> FromRequest<'a> for TakoTypedMultipart<'a, T, F>
where
  T: DeserializeOwned + 'static,
  F: FromMultipartField + serde::Serialize + 'static,
{
  type Error = TypedMultipartError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .ok_or(TypedMultipartError::MissingContentType)?;

      let content_type_str = content_type
        .to_str()
        .map_err(|_| TypedMultipartError::InvalidUtf8)?;

      let boundary = multer::parse_boundary(content_type_str)
        .map_err(|e| TypedMultipartError::BoundaryParseError(e.to_string()))?;

      let cfg = MultipartConfig::lookup(req.extensions());
      let constraints = cfg.to_constraints();
      let mut multipart =
        Multipart::with_constraints(req.body_mut().into_data_stream(), boundary, constraints);
      let mut map = Map::<String, Value>::new();
      let mut count: usize = 0;

      let field_timeout = cfg.field_chunk_timeout;
      loop {
        let next_field_fut = multipart.next_field();
        let field = match field_timeout {
          Some(d) => match tokio::time::timeout(d, next_field_fut).await {
            Ok(Ok(field)) => field,
            Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
            Err(_) => {
              return Err(TypedMultipartError::FieldError(
                "multipart slow-read timeout".to_string(),
              ));
            }
          },
          None => next_field_fut
            .await
            .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
        };
        let Some(field) = field else {
          break;
        };
        count += 1;
        if let Some(max) = cfg.max_parts
          && count > max
        {
          return Err(TypedMultipartError::TooManyParts);
        }
        let part_ct = field.content_type().map(std::string::ToString::to_string);
        if !cfg.content_type_ok(part_ct.as_deref()) {
          return Err(TypedMultipartError::DisallowedContentType(
            part_ct.unwrap_or_default(),
          ));
        }

        let field_name = field
          .name()
          .ok_or_else(|| TypedMultipartError::FieldError("Field name missing".to_string()))?
          .to_owned();

        if field.file_name().is_some() {
          let file_value: F = match field_timeout {
            Some(d) => match tokio::time::timeout(d, F::from_field(field)).await {
              Ok(Ok(v)) => v,
              Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
              Err(_) => {
                return Err(TypedMultipartError::FieldError(
                  "multipart slow-read timeout".to_string(),
                ));
              }
            },
            None => F::from_field(field)
              .await
              .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
          };

          let json_value = serde_json::to_value(file_value)
            .map_err(|e| TypedMultipartError::DeserializationError(e.to_string()))?;

          map.insert(field_name, json_value);
        } else {
          let field_bytes = match field_timeout {
            Some(d) => match tokio::time::timeout(d, field.bytes()).await {
              Ok(Ok(b)) => b,
              Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
              Err(_) => {
                return Err(TypedMultipartError::FieldError(
                  "multipart slow-read timeout".to_string(),
                ));
              }
            },
            None => field
              .bytes()
              .await
              .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
          };

          let text = String::from_utf8(field_bytes.to_vec())
            .map_err(|_| TypedMultipartError::InvalidUtf8)?;

          map.insert(field_name, Value::String(text));
        }
      }

      let data: T = serde_json::from_value(Value::Object(map))
        .map_err(|e| TypedMultipartError::DeserializationError(e.to_string()))?;

      Ok(Self {
        data,
        _marker: core::marker::PhantomData,
      })
    }
  }
}
