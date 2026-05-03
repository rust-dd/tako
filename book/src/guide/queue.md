# Queue

> **Status:** scaffold.

Tako ships an in-process job queue at `tako_core::queue` with a
`QueueBackend` trait so remote brokers can plug in. Built-ins:

- `MemoryBackend` — in-process default.
- `Queue::push_dedup(name, payload, key)` — collapses duplicate
  pending jobs.
- `tako_core::queue::cron::CronScheduler` (feature `queue-cron`).

Signals fired per job: `queue.job.queued`, `started`, `completed`,
`failed`, `retrying`, `dead_letter`. Canonical strings live under
`tako_core::queue::signal_ids`.

> Companion crates `tako-stores-redis` and `tako-stores-postgres` are
> on the follow-up list — they will implement `QueueBackend` plus the
> session / rate-limit / idempotency / JWKS / CSRF stores.
