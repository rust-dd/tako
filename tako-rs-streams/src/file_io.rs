use std::io;
use std::path::Path;
use std::time::SystemTime;

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tako_rs_core::body::TakoBody;

pub(crate) mod conditional;
pub(crate) mod date;
pub(crate) mod etag;
pub(crate) mod range;
pub(crate) mod response;

/// Bounded chunks of an open file; with `compio`, poll and drop it on the thread that opened it.
pub type FileByteStream = BoxStream<'static, io::Result<Bytes>>;

/// Files up to this size are read by the blocking call that opens them.
#[cfg(not(feature = "compio"))]
const INLINE_READ_MAX: u64 = 128 * 1024;

pub(crate) struct OpenFile {
  source: Source,
  pub(crate) size: u64,
  pub(crate) modified: Option<SystemTime>,
}

enum Source {
  #[cfg(not(feature = "compio"))]
  File(tokio::fs::File),
  #[cfg(not(feature = "compio"))]
  Read(Bytes),
  #[cfg(feature = "compio")]
  File(send_wrapper::SendWrapper<compio::fs::File>),
}

impl OpenFile {
  #[cfg(any(feature = "compio", feature = "file-stream"))]
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
        source: Source::File(file),
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
        source: Source::File(send_wrapper::SendWrapper::new(file)),
        size: metadata.len(),
        modified: metadata.modified().ok(),
      })
    })
    .await
  }

  /// Opens `path` on a blocking thread and reads it whole when it is small.
  #[cfg(not(feature = "compio"))]
  pub(crate) fn open_blocking(path: &Path) -> io::Result<Self> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
      return Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "not a regular file",
      ));
    }
    let modified = metadata.modified().ok();
    if metadata.len() > INLINE_READ_MAX {
      return Ok(Self {
        source: Source::File(tokio::fs::File::from_std(file)),
        size: metadata.len(),
        modified,
      });
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.take(metadata.len()).read_to_end(&mut contents)?;
    Ok(Self {
      size: contents.len() as u64,
      source: Source::Read(Bytes::from(contents)),
      modified,
    })
  }

  /// The body for `length` bytes from `start`; a file already read becomes one buffer.
  pub(crate) async fn into_body(self, start: u64, length: u64) -> io::Result<TakoBody> {
    #[cfg(not(feature = "compio"))]
    if let Source::Read(contents) = self.source {
      return Ok(TakoBody::from(
        contents.slice(start as usize..(start + length) as usize),
      ));
    }
    Ok(TakoBody::from_stream(
      self.into_stream(start, length).await?,
    ))
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
    match self.source {
      Source::File(mut file) => {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncSeekExt;
        if start != 0 {
          file.seek(std::io::SeekFrom::Start(start)).await?;
        }
        Ok(tokio_util::io::ReaderStream::with_capacity(file.take(length), 64 * 1024).boxed())
      }
      Source::Read(contents) => {
        let range = contents.slice(start as usize..(start + length) as usize);
        Ok(futures_util::stream::once(std::future::ready(Ok(range))).boxed())
      }
    }
    #[cfg(feature = "compio")]
    {
      use compio::io::AsyncReadAt;
      let Source::File(file) = self.source;
      let stream =
        futures_util::stream::try_unfold((file, start, length), |(file, offset, remaining)| {
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
        });
      Ok(stream.boxed())
    }
  }
}
