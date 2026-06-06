import Link from 'next/link';

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col items-center justify-center px-6 py-20 text-center">
      <p className="mb-4 font-mono text-sm uppercase tracking-widest text-fd-muted-foreground">
        HTTP · WebSocket · SSE · gRPC · HTTP/3 · TCP/UDP
      </p>
      <h1 className="text-5xl font-bold tracking-tight sm:text-6xl">🐙 tako</h1>
      <p className="mt-6 max-w-2xl text-lg text-fd-muted-foreground">
        A pragmatic, ergonomic, multi-transport Rust framework for modern
        network services. Build one cohesive application across HTTP/1.1,
        HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and
        WebTransport — with a single routing, middleware, and observability
        model, on Tokio or Compio.
      </p>

      <div className="mt-10 flex flex-wrap items-center justify-center gap-4">
        <Link
          href="/docs"
          className="rounded-lg bg-fd-primary px-5 py-3 text-sm font-medium text-fd-primary-foreground shadow hover:opacity-90"
        >
          Read the docs
        </Link>
        <Link
          href="/docs/getting-started/quickstart"
          className="rounded-lg border border-fd-border px-5 py-3 text-sm font-medium hover:bg-fd-muted"
        >
          Quickstart
        </Link>
        <a
          href="https://crates.io/crates/tako-rs"
          className="rounded-lg border border-fd-border px-5 py-3 text-sm font-medium hover:bg-fd-muted"
        >
          crates.io
        </a>
      </div>

      <section className="mt-24 grid w-full max-w-5xl grid-cols-1 gap-6 text-left sm:grid-cols-2 lg:grid-cols-3">
        <Feature
          title="Multi-transport by design"
          body="HTTP/1.1, HTTP/2, HTTP/3 (QUIC), WebSocket, WebTransport, SSE, gRPC, raw TCP / UDP, Unix sockets, and PROXY protocol — all behind one Router and one middleware stack."
        />
        <Feature
          title="Two runtimes, one model"
          body="The same framework style on Tokio or Compio, including TLS and HTTP/2 on both sides where supported. Pick the runtime that fits the deployment, keep the code."
        />
        <Feature
          title="Typed extraction"
          body="22+ extractors for JSON (with optional SIMD), form, query, path, headers, cookies, JWT claims, API keys, Accept, Range, protobuf, and multipart. NumPy-free, just Rust types in and out."
        />
        <Feature
          title="Middleware & auth included"
          body="JWT / Basic / Bearer / API-key auth, CSRF, sessions, security headers, request IDs, body limits, rate limiting, CORS, idempotency, and compression — part of the framework, not glue."
        />
        <Feature
          title="Realtime-ready"
          body="Streaming responses, SSE, WebSockets, GraphQL subscriptions, HTTP/3, and WebTransport under one crate. Background queue, in-process signals, and graceful shutdown ship in the box."
        />
        <Feature
          title="Performance paths"
          body="SIMD JSON (sonic-rs / simd-json), optional zero-copy extractors, brotli / gzip / deflate / zstd compression, and jemalloc support — without fragmenting the API."
        />
      </section>
    </main>
  );
}

function Feature({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-xl border border-fd-border bg-fd-card p-6 shadow-sm">
      <h2 className="font-semibold">{title}</h2>
      <p className="mt-2 text-sm text-fd-muted-foreground">{body}</p>
    </div>
  );
}
