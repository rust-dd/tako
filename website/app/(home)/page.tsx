import Link from 'next/link';
import type { Metadata } from 'next';
import { ServerCodeBlock } from 'fumadocs-ui/components/codeblock.rsc';
import { JsonLd } from '@/components/JsonLd';
import { repoUrl, siteDescription, siteName, siteUrl } from '@/lib/site';
import { crateMinorVersion, crateVersion } from '@/lib/workspace';

export const metadata: Metadata = {
  alternates: { canonical: '/' },
};

const cargoToml = `[dependencies]
tako-rs = "${crateMinorVersion}"
tokio = { version = "1", features = ["macros", "net", "rt-multi-thread"] }`;

const mainRs = `use tako::{router::Router, types::BoxError, Server};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let mut router = Router::new();
    router.get("/", || async { "Hello, Tako!" });

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    Server::builder()
        .build()
        .try_spawn_http(listener, router)?
        .result()
        .await?;

    Ok(())
}`;

const codeThemes = { light: 'github-light', dark: 'github-dark' } as const;

const faqs: { question: string; answer: string; link?: { href: string; label: string } }[] = [
  {
    question: 'Is the crate called tako or tako-rs?',
    answer:
      'The crates.io package is tako-rs, and the library it provides is imported as tako (use tako::…). The separate tako crate on crates.io is an unrelated project.',
  },
  {
    question: 'How does Tako compare to Axum and Actix Web?',
    answer:
      'Axum and Actix Web are mature HTTP frameworks with larger ecosystems. Tako focuses on running several transports (HTTP/3, WebSocket, SSE, gRPC, raw sockets) behind one router, with auth, sessions, rate limiting, and metrics built in.',
    link: { href: '/docs/concepts/comparison', label: 'Read the comparison' },
  },
  {
    question: 'Which async runtimes does Tako support?',
    answer:
      'Tokio by default, and Compio (io_uring on Linux, IOCP on Windows) through the compio feature. HTTP/3, WebTransport, Unix sockets, and PROXY protocol are Tokio-only.',
    link: { href: '/docs/concepts/runtimes', label: 'Runtime compatibility' },
  },
  {
    question: 'Does Tako have a thread-per-core mode?',
    answer:
      'Yes. The per-thread feature runs the router on one current-thread runtime per core with SO_REUSEPORT and balances connections across workers. In the published benchmarks it lands within a few percent of Actix Web and ntex.',
    link: { href: '/docs/deployment#thread-per-core', label: 'Thread-per-core deployment' },
  },
  {
    question: 'Which Rust version does Tako need?',
    answer: 'Rust 1.95 or newer. The workspace uses edition 2024.',
  },
  {
    question: 'Is there an LLM-friendly version of the docs?',
    answer:
      'Yes. /llms.txt indexes every page, /llms-full.txt holds the whole guide in one Markdown file, and any docs page is available as Markdown by appending .mdx to its URL.',
    link: { href: '/docs/getting-started/ai-assistants', label: 'Using Tako with AI assistants' },
  },
];

const structuredData = {
  '@context': 'https://schema.org',
  '@graph': [
    {
      '@type': 'WebSite',
      '@id': `${siteUrl}/#website`,
      name: siteName,
      alternateName: ['tako-rs', 'Tako Rust web framework'],
      url: siteUrl,
    },
    {
      '@type': 'SoftwareSourceCode',
      '@id': `${siteUrl}/#software`,
      name: siteName,
      alternateName: 'tako-rs',
      description: siteDescription,
      url: siteUrl,
      codeRepository: repoUrl,
      programmingLanguage: { '@type': 'ComputerLanguage', name: 'Rust' },
      runtimePlatform: ['Tokio', 'Compio'],
      license: 'https://opensource.org/licenses/MIT',
      version: crateVersion,
      author: { '@type': 'Organization', name: 'rust-dd', url: 'https://rust-dd.com' },
      sameAs: ['https://crates.io/crates/tako-rs', 'https://docs.rs/tako-rs', repoUrl],
    },
    {
      '@type': 'FAQPage',
      mainEntity: faqs.map(({ question, answer }) => ({
        '@type': 'Question',
        name: question,
        acceptedAnswer: { '@type': 'Answer', text: answer },
      })),
    },
  ],
};

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col items-center px-6 py-20">
      <JsonLd data={structuredData} />
      <section className="flex max-w-3xl flex-col items-center text-center">
        <p className="mb-4 font-mono text-sm uppercase tracking-widest text-fd-muted-foreground">
          HTTP · WebSocket · SSE · gRPC · HTTP/3 · TCP/UDP
        </p>
        <h1 className="text-5xl font-bold tracking-tight sm:text-6xl">🐙 tako</h1>
        <p className="mt-6 text-lg text-fd-muted-foreground">
          Tako is a Rust web framework for services that speak more than HTTP.
          One router, one middleware stack, and one observability model cover
          HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix
          sockets, and WebTransport, on Tokio or Compio.
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
      </section>

      <section className="mt-24 w-full max-w-3xl">
        <h2 className="text-2xl font-semibold">Hello, Tako</h2>
        <p className="mt-2 text-fd-muted-foreground">
          The package is <code>tako-rs</code>; the Rust import is <code>tako</code>.
          Add it with <code>cargo add tako-rs</code> or in <code>Cargo.toml</code>:
        </p>
        <div className="mt-4 flex flex-col gap-4">
          <ServerCodeBlock code={cargoToml} lang="toml" themes={codeThemes} codeblock={{ title: 'Cargo.toml' }} />
          <ServerCodeBlock code={mainRs} lang="rust" themes={codeThemes} codeblock={{ title: 'src/main.rs' }} />
        </div>
        <p className="mt-4 text-sm text-fd-muted-foreground">
          Run it with <code>cargo run</code>, then <code>curl http://127.0.0.1:8080/</code>.{' '}
          <Link href="/docs/getting-started/quickstart" className="underline">
            Continue with the Quickstart
          </Link>
          .
        </p>
      </section>

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
          body="22+ extractors for JSON (with optional SIMD), form, query, path, headers, cookies, JWT claims, API keys, Accept, Range, protobuf, and multipart."
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

      <section className="mt-24 w-full max-w-3xl">
        <h2 className="text-2xl font-semibold">Benchmarks</h2>
        <p className="mt-2 text-fd-muted-foreground">
          Hello-world requests per second at 100 and 1,000 connections, measured
          with tako-rs 2.2.0 in a 24 vCPU Linux container. The thread-per-core
          server lands within a few percent of Actix Web and ntex; the default
          server keeps pace with Axum and pulls ahead at 1,000 connections.
        </p>
        <img
          src="/benchmarks/hello-world-light.svg"
          alt="Hello-world requests per second at 100 and 1,000 connections for Actix Web, Tako per-thread, ntex, Tako with jemalloc, Axum, and Tako"
          width={896}
          height={306}
          className="mt-6 w-full dark:hidden"
        />
        <img
          src="/benchmarks/hello-world-dark.svg"
          alt="Hello-world requests per second at 100 and 1,000 connections for Actix Web, Tako per-thread, ntex, Tako with jemalloc, Axum, and Tako"
          width={896}
          height={306}
          className="mt-6 hidden w-full dark:block"
        />
        <p className="mt-4 text-sm text-fd-muted-foreground">
          Results move with hardware and configuration.{' '}
          <Link href="/docs/benchmarks" className="underline">
            Methodology and reproduction steps
          </Link>
          .
        </p>
      </section>

      <section className="mt-24 w-full max-w-3xl">
        <h2 className="text-2xl font-semibold">FAQ</h2>
        <dl className="mt-6 flex flex-col gap-6">
          {faqs.map(({ question, answer, link }) => (
            <div key={question}>
              <dt className="font-semibold">{question}</dt>
              <dd className="mt-2 text-fd-muted-foreground">
                {answer}
                {link ? (
                  <>
                    {' '}
                    <Link href={link.href} className="underline">
                      {link.label}
                    </Link>
                    .
                  </>
                ) : null}
              </dd>
            </div>
          ))}
        </dl>
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
