use std::sync::Arc;

pub(crate) struct BoundEndpoint<E> {
  pub(crate) endpoint: E,
  pub(crate) connections: ConnectionConfig,
}

pub(crate) struct ConnectionConfig {
  base: Arc<quinn::ServerConfig>,
  #[cfg(feature = "http3-qlog")]
  qlog: Option<super::qlog::Qlog>,
  #[cfg(feature = "http3-qlog")]
  settings: crate::ServerConfig,
}

impl ConnectionConfig {
  #[cfg_attr(not(feature = "http3-qlog"), allow(clippy::unnecessary_wraps))]
  pub(crate) fn new(
    base: quinn::ServerConfig,
    #[cfg(feature = "http3-qlog")] directory: Option<&std::path::Path>,
    #[cfg(feature = "http3-qlog")] settings: &crate::ServerConfig,
  ) -> std::io::Result<Self> {
    Ok(Self {
      base: Arc::new(base),
      #[cfg(feature = "http3-qlog")]
      qlog: directory.map(super::qlog::Qlog::new).transpose()?,
      #[cfg(feature = "http3-qlog")]
      settings: settings.clone(),
    })
  }

  pub(crate) fn base(&self) -> quinn::ServerConfig {
    self.base.as_ref().clone()
  }

  pub(crate) fn for_connection(&self) -> Arc<quinn::ServerConfig> {
    #[cfg(feature = "http3-qlog")]
    if let Some(qlog) = &self.qlog {
      let mut config = self.base();
      let mut transport = super::config::transport_config_from(&self.settings);
      transport.qlog_stream(qlog.stream());
      config.transport_config(Arc::new(transport));
      return Arc::new(config);
    }
    self.base.clone()
  }

  #[cfg(feature = "http3-qlog")]
  pub(crate) async fn finish(self) {
    if let Some(qlog) = self.qlog {
      qlog.finish().await;
    }
  }
}
