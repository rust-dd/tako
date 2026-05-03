# Observability

> **Status:** scaffold.

- **Logs**: `AccessLog` middleware emits one structured line per
  request. Default sink is `tracing` at INFO; custom sink hook for
  JSON / OTLP / file rotation.
- **Metrics**: Prometheus middleware at
  `tako_plugins::plugins::metrics_prometheus` (feature
  `metrics-prometheus`). Latency histogram with
  `PrometheusMetricsConfig::with_buckets(..)` override. Matched-route
  label is bounded to keep cardinality finite. OTLP via
  `metrics-opentelemetry`.
- **Tracing**: `Traceparent` middleware parses W3C Trace Context and
  emits a `TraceContext` extension; outbound spans propagate
  automatically.
- **Health**: `Healthcheck` middleware exposes `/live`, `/ready`,
  `/__drain`. `HealthcheckHandle::drain()` flips the gate from a
  SIGTERM handler.

> HTTP/3 qlog and `traceparent` propagation through the v2 outbound
> client are deferred follow-up items.
