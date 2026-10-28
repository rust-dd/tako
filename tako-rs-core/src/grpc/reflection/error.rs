use std::fmt;

#[derive(Debug)]
pub enum DescriptorError {
  Decode(prost::DecodeError),
  MissingFileName,
}

impl fmt::Display for DescriptorError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(error) => error.fmt(formatter),
      Self::MissingFileName => formatter.write_str("file descriptor has no name"),
    }
  }
}

impl std::error::Error for DescriptorError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Decode(error) => Some(error),
      Self::MissingFileName => None,
    }
  }
}

impl From<prost::DecodeError> for DescriptorError {
  fn from(error: prost::DecodeError) -> Self {
    Self::Decode(error)
  }
}
