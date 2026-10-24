# GraphQL JSON buffering allocations

The JSON extractor now deserializes the bounded `Bytes` directly. Multipart
requests retain async-graphql's multipart parser, and raw `application/graphql`
requests construct one query from UTF-8 text.

Reproduce the allocation comparison with:

```sh
CARGO_INCREMENTAL=0 cargo test -p tako-rs-core --features async-graphql --test graphql_allocations -- --nocapture
```

The fixture is `{"query":"aaaa..."}` with 1 KiB, 64 KiB, or 512 KiB of query
text. Both paths start with the same prebuffered `Bytes`; fixture construction
is outside the measured region. The baseline collects with `Limited` and calls
async-graphql's `receive_batch_body`, which reads the bytes into a second `Vec`.
The candidate calls Tako's public `receive_graphql_batch`.

A thread-local counter around the system allocator records allocation and
reallocation calls and their requested byte sizes. These totals count each
reallocation request, so they are neither peak nor retained memory. The futures
complete without scheduling work on another thread. The baseline omits Tako's
small content-type checks, making the measured savings conservative.

Measured on macOS arm64 with Rust 1.99.0 and async-graphql 7.2.1:

| JSON bytes | Baseline allocation calls | Direct allocation calls | Baseline requested bytes | Direct requested bytes |
| ---: | ---: | ---: | ---: | ---: |
| 1,036 | 11 | 6 | 5,488 | 1,456 |
| 65,548 | 17 | 6 | 328,048 | 65,968 |
| 524,300 | 20 | 6 | 2,621,808 | 524,720 |

The regression requires at least one body-sized reduction for every fixture.
It does not assert exact allocator counts or claim a throughput improvement.
Separate contract tests cover single requests, batches, malformed JSON, MIME
parameter errors, transport errors, raw queries, multipart options, and the
shared and protocol-specific size caps.
