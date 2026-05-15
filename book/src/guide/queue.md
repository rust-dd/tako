# Queue

Tako ships an in-process background job queue at `tako::queue`. It is
designed for the "fire-and-forget" workloads that usually end up
hand-rolled on top of `tokio::spawn` — send-email, dispatch-webhook,
reindex, etc. — with retry, dead-lettering, deduplication, and
delayed execution built in.

```rust
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tako::Method;
use tako::extractors::json::Json;
use tako::extractors::state::State;
use tako::queue::{Job, Queue, RetryPolicy};
use tako::responder::Responder;
use tako::router::Router;

#[derive(Serialize, Deserialize)]
struct Email { to: String, subject: String }

async fn enqueue(
  State(q): State<Queue>,
  Json(req): Json<Email>,
) -> impl Responder {
  match q.0.push("send_email", &req).await {
    Ok(id) => (tako::StatusCode::ACCEPTED, format!("queued id={id}\n")),
    Err(e) => (tako::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let queue = Queue::builder()
    .workers(4)
    .retry(RetryPolicy::exponential(3, Duration::from_millis(500)))
    .build();

  queue.register("send_email", |job: Job| async move {
    let payload: Email = job.deserialize()?;
    println!("send_email -> {} / {}", payload.to, payload.subject);
    Ok(())
  });
  queue.start();

  let mut router = Router::new();
  router.with_state(queue);
  router.route(Method::POST, "/emails", enqueue);

  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
  tako::serve(listener, router).await;
  Ok(())
}
```

The builder gives you a fluent way to set worker count and retry
policy. Job handlers are registered by name with
`queue.register(name, handler)`; each handler returns `Result<(),
QueueError>`. Push jobs with `queue.push(name, payload)`,
`queue.push_delayed(name, payload, duration)`, or
`queue.push_dedup(name, payload, key)` to collapse duplicate pending
jobs by an idempotency key.

Operational helpers:

- `queue.pending_count()`, `queue.inflight_count()` — gauge-style
  metrics for dashboards.
- `queue.dead_letters()` — read the dead-letter queue. Jobs that
  exhaust their retry budget land here for manual inspection.
- `queue.shutdown(timeout)` — drain workers gracefully on SIGTERM.
- `tako::queue::cron::CronScheduler` (feature `queue-cron`) — wire
  a crontab spec into `queue.push`.

Signals are emitted for every job lifecycle event:

- `queue.job.queued`, `.started`, `.completed`, `.failed`,
  `.retrying`, `.dead_letter`

The canonical strings live under `tako::queue::signal_ids`. They are
part of the stable signal contract — see
[API stability](../reference/stability.md).

The default `MemoryBackend` keeps everything in process. Companion
crates `tako-stores-redis` and `tako-stores-postgres` are on the
follow-up list and will implement `QueueBackend` plus the session /
rate-limit / idempotency / JWKS / CSRF stores.

See [`examples/job-queue`](https://github.com/rust-dd/tako/tree/main/examples/job-queue)
for a full end-to-end demo including retries, delayed jobs, and the
dead-letter queue.
