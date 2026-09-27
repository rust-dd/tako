use std::io;
use std::path::Path;
use std::time::SystemTime;

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;

pub(crate) mod conditional;
pub(crate) mod date;
pub(crate) mod etag;
pub(crate) mod range;
pub(crate) mod response;

/// A bounded byte stream read from an open file.
///
/// With `compio`, poll and drop the stream on the thread that opened it.
pub type FileByteStream = BoxStream<'static, io::Result<Bytes>>;

pub(crate) struct OpenFile {
  #[cfg(not(feature = "compio"))]
  file: tokio::fs::File,
  #[cfg(feature = "compio")]
  file: send_wrapper::SendWrapper<compio::fs::File>,
  pub(crate) size: u64,
  pub(crate) modified: Option<SystemTime>,
}

impl OpenFile {
  pub(crate) async fn open(path: &Path) -> io::Result<Self> {
    #[cfg(not(feature = "compio"))]
    {
      let file = tokio::fs::File::open(path).await?;
      let metadata = file.metadata().await?;
      if !metadata.is_file() {
        return Err(io::Error::new(
          io::ErrorKind::InvalidInput,
          "not a regular file",
        ));
      }
      Ok(Self {
        file,
        size: metadata.len(),
        modified: metadata.modified().ok(),
      })
    }
    #[cfg(feature = "compio")]
    send_wrapper::SendWrapper::new(async {
      let file = compio::fs::File::open(path).await?;
      let metadata = file.metadata().await?;
      if !metadata.is_file() {
        return Err(io::Error::new(
          io::ErrorKind::InvalidInput,
          "not a regular file",
        ));
      }
      Ok(Self {
        file: send_wrapper::SendWrapper::new(file),
        size: metadata.len(),
        modified: metadata.modified().ok(),
      })
    })
    .await
  }

  #[cfg_attr(
    feature = "compio",
    allow(
      clippy::unused_async_trait_impl,
      reason = "The shared async interface seeks on Tokio; positional Compio reads need no seek"
    )
  )]
  pub(crate) async fn into_stream(self, start: u64, length: u64) -> io::Result<FileByteStream> {
    #[cfg(not(feature = "compio"))]
    {
      use tokio::io::AsyncReadExt;
      use tokio::io::AsyncSeekExt;
      let mut file = self.file;
      if start != 0 {
        file.seek(std::io::SeekFrom::Start(start)).await?;
      }
      Ok(tokio_util::io::ReaderStream::with_capacity(file.take(length), 64 * 1024).boxed())
    }
    #[cfg(feature = "compio")]
    {
      use compio::io::AsyncReadAt;
      let stream = futures_util::stream::try_unfold(
        (self.file, start, length),
        |(file, offset, remaining)| {
          send_wrapper::SendWrapper::new(async move {
            if remaining == 0 {
              return Ok(None);
            }
            let buffer = Vec::with_capacity(remaining.min(64 * 1024) as usize);
            let compio::BufResult(read, buffer) = file.read_at(buffer, offset).await;
            let read = read?;
            if read == 0 {
              return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "file truncated during transfer",
              ));
            }
            Ok(Some((
              Bytes::from(buffer),
              (file, offset + read as u64, remaining - read as u64),
            )))
          })
        },
      );
      Ok(stream.boxed())
    }
  }
}
