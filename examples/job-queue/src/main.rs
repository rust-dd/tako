use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use tako::Method;
use tako::extractors::json::Json;
use tako::extractors::state::State;
use tako::queue::Job;
use tako::queue::Queue;
use tako::queue::QueueError;
use tako::queue::RetryPolicy;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

#[derive(Debug, Serialize, Deserialize)]
struct EmailPayload {
  to: String,
  subject: String,
  body: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebhookPayload {
  url: String,
  event: String,
}

#[derive(Deserialize)]
struct SendEmailRequest {
  to: String,
  subject: String,
  body: String,
}

async fn send_email_handler(
  queue: State<Queue>,
  Json(req): Json<SendEmailRequest>,
) -> impl Responder {
  let payload = EmailPayload {
    to: req.to,
    subject: req.subject,
    body: req.body,
  };

  match queue.0.push("send_email", &payload).await {
    Ok(id) => (
      tako::StatusCode::ACCEPTED,
      format!("Email job queued (id: {id})\n"),
    ),
    Err(e) => (
      tako::StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to queue: {e}\n"),
    ),
  }
}

#[derive(Deserialize)]
struct WebhookRequest {
  url: String,
  event: String,
}

async fn send_webhook_handler(
  queue: State<Queue>,
  Json(req): Json<WebhookRequest>,
) -> impl Responder {
  let payload = WebhookPayload {
    url: req.url,
    event: req.event,
  };

  match queue.0.push("send_webhook", &payload).await {
    Ok(id) => (
      tako::StatusCode::ACCEPTED,
      format!("Webhook job queued (id: {id})\n"),
    ),
    Err(e) => (
      tako::StatusCode::INTERNAL_SERVER_ERROR,
      format!("Failed to queue: {e}\n"),
    ),
  }
}

async fn delayed_handler(queue: State<Queue>) -> impl Responder {
  match queue
    .0
    .push_delayed(
      "send_email",
      &EmailPayload {
        to: "delayed@example.com".into(),
        subject: "Delayed email".into(),
        body: "This was sent after a 5s delay".into(),
      },
      Duration::from_secs(5),
    )
    .await
  {
    Ok(id) => format!("Delayed job queued (id: {id}), will run in 5s\n"),
    Err(e) => format!("Failed: {e}\n"),
  }
}

async fn stats_handler(queue: State<Queue>) -> impl Responder {
  let pending = queue.0.pending_count();
  let inflight = queue.0.inflight_count();
  let dlq = queue.0.dead_letters();

  let mut resp = format!(
    "Queue stats:\n  pending: {pending}\n  inflight: {inflight}\n  dead letters: {}\n",
    dlq.len()
  );

  if !dlq.is_empty() {
    resp.push_str("\nDead letter queue:\n");
    for dj in &dlq {
      resp.push_str(&format!(
        "  [id={}] {} — attempts: {}, error: {}\n",
        dj.id, dj.name, dj.attempts, dj.error
      ));
    }
  }

  resp
}

async fn fail_handler(queue: State<Queue>) -> impl Responder {
  match queue.0.push("always_fail", &"trigger failure").await {
    Ok(id) => format!("Failing job queued (id: {id}), check /stats after a few seconds\n"),
    Err(e) => format!("Failed: {e}\n"),
  }
}

async fn health(_: Request) -> impl Responder {
  "ok"
}

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  // Build the queue: 4 workers, exponential backoff retry (max 3 retries)
  let queue = Queue::builder()
    .workers(4)
    .retry(RetryPolicy::exponential(3, Duration::from_millis(500)))
    .build();

  // Register job handlers
  queue.register("send_email", |job: Job| async move {
    let email: EmailPayload = job.deserialize()?;
    println!(
      "[email] Sending to: {} | Subject: {} (attempt {})",
      email.to, email.subject, job.attempt
    );
    // Simulate sending...
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!("[email] Sent to {}", email.to);
    Ok(())
  });

  queue.register("send_webhook", |job: Job| async move {
    let webhook: WebhookPayload = job.deserialize()?;
    println!(
      "[webhook] POST {} event={} (attempt {})",
      webhook.url, webhook.event, job.attempt
    );
    // Simulate HTTP call...
    tokio::time::sleep(Duration::from_millis(50)).await;
    println!("[webhook] Delivered to {}", webhook.url);
    Ok(())
  });

  queue.register("always_fail", |job: Job| async move {
    println!("[always_fail] attempt {} — about to fail", job.attempt);
    Err(QueueError::HandlerError(
      "simulated permanent failure".into(),
    ))
  });

  // Start background workers
  queue.start();

  // Store queue in global state so handlers can access it
  tako::state::set_state(queue.clone());

  // Build router
  let mut router = Router::new();
  router.route(Method::POST, "/email", send_email_handler);
  router.route(Method::POST, "/webhook", send_webhook_handler);
  router.route(Method::POST, "/delayed", delayed_handler);
  router.route(Method::POST, "/fail", fail_handler);
  router.route(Method::GET, "/stats", stats_handler);
  router.route(Method::GET, "/health", health);

  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;

  println!("Job queue server on http://127.0.0.1:8080");
  println!();
  println!("Endpoints:");
  println!("  POST /email    - Queue an email job");
  println!("  POST /webhook  - Queue a webhook job");
  println!("  POST /delayed  - Queue a delayed email (5s)");
  println!("  POST /fail     - Queue a job that always fails (tests retry + DLQ)");
  println!("  GET  /stats    - View queue stats and dead letters");
  println!("  GET  /health   - Health check");
  println!();
  println!("Examples:");
  println!(
    r#"  curl -X POST -H "Content-Type: application/json" -d '{{"to":"user@example.com","subject":"Hello","body":"World"}}' http://127.0.0.1:8080/email"#
  );
  println!(
    r#"  curl -X POST -H "Content-Type: application/json" -d '{{"url":"https://hook.example.com","event":"order.created"}}' http://127.0.0.1:8080/webhook"#
  );
  println!("  curl -X POST http://127.0.0.1:8080/fail");
  println!("  curl http://127.0.0.1:8080/stats");

  tako::serve(listener, router).await;

  // Graceful shutdown
  queue.shutdown(Duration::from_secs(5)).await;

  Ok(())
}
