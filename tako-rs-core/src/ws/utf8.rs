use super::ProtocolError;

/// Validates UTF-8 across message fragments, carrying a character cut at a fragment end.
#[derive(Default)]
pub(crate) struct Utf8Validator {
  tail: [u8; 4],
  tail_len: u8,
}

impl Utf8Validator {
  pub(crate) fn feed(&mut self, mut chunk: &[u8]) -> Result<(), ProtocolError> {
    if self.tail_len > 0 {
      let held = usize::from(self.tail_len);
      let width = char_width(self.tail[0]).ok_or(ProtocolError::InvalidUtf8)?;
      let take = (width - held).min(chunk.len());
      self.tail[held..held + take].copy_from_slice(&chunk[..take]);
      match std::str::from_utf8(&self.tail[..held + take]) {
        Ok(_) => self.tail_len = 0,
        Err(error) if error.error_len().is_none() => {
          self.tail_len = (held + take) as u8;
          return Ok(());
        }
        Err(_) => return Err(ProtocolError::InvalidUtf8),
      }
      chunk = &chunk[take..];
    }
    match std::str::from_utf8(chunk) {
      Ok(_) => Ok(()),
      Err(error) if error.error_len().is_none() => {
        let rest = &chunk[error.valid_up_to()..];
        self.tail[..rest.len()].copy_from_slice(rest);
        self.tail_len = rest.len() as u8;
        Ok(())
      }
      Err(_) => Err(ProtocolError::InvalidUtf8),
    }
  }

  pub(crate) fn finish(&self) -> Result<(), ProtocolError> {
    if self.tail_len == 0 {
      Ok(())
    } else {
      Err(ProtocolError::InvalidUtf8)
    }
  }
}

fn char_width(lead: u8) -> Option<usize> {
  match lead {
    0xC2..=0xDF => Some(2),
    0xE0..=0xEF => Some(3),
    0xF0..=0xF4 => Some(4),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::Utf8Validator;

  const TEXT: &str = "héllo wörld €𝄞";

  #[test]
  fn valid_text_passes_in_one_byte_pieces() {
    let mut validator = Utf8Validator::default();
    for byte in TEXT.as_bytes() {
      validator.feed(std::slice::from_ref(byte)).unwrap();
    }
    validator.finish().unwrap();
  }

  #[test]
  fn valid_text_passes_at_every_split_point() {
    let bytes = TEXT.as_bytes();
    for split in 0..=bytes.len() {
      let mut validator = Utf8Validator::default();
      validator.feed(&bytes[..split]).unwrap();
      validator.feed(&bytes[split..]).unwrap();
      validator.finish().unwrap();
    }
  }

  #[test]
  fn a_truncated_character_fails_at_the_end() {
    let mut validator = Utf8Validator::default();
    validator.feed(&[0xE2, 0x82]).unwrap();
    assert!(validator.finish().is_err());
  }

  #[test]
  fn invalid_sequences_fail_in_the_piece_that_carries_them() {
    assert!(Utf8Validator::default().feed(&[0xC0, 0x80]).is_err());
    assert!(Utf8Validator::default().feed(&[0xF4, 0x90]).is_err());
    let mut surrogate = Utf8Validator::default();
    surrogate.feed(&[0xED]).unwrap();
    assert!(surrogate.feed(&[0xA0, 0x80]).is_err());
  }
}
