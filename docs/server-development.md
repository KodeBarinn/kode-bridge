# Server Development Notes

This document supplements the [Server Guide](../SERVER_GUIDE.md) with the
current kode-bridge 0.5 handler, lifecycle, and testing contracts. It avoids
duplicating the complete runnable examples in `examples/`.

## Handler contract

A `Router` handler is a `Send + Sync + 'static` function that returns a `Send`
future with `kode_bridge::Result<HttpResponse>`.

```rust
let router = kode_bridge::Router::new()
    .put("/config/:section", |ctx| async move {
        let section = ctx
            .path_params
            .get("section")
            .cloned()
            .unwrap_or_default();
        let value: serde_json::Value = ctx.json()?;
        kode_bridge::HttpResponse::json(&serde_json::json!({
            "section": section,
            "value": value,
        }))
    });
```

The router supports literal paths and `:name` parameters. It rejects traversal,
backslashes, control characters, paths without a leading slash, and paths over
2048 bytes before dispatch.

There is no middleware registry in version 0.5. Share behavior through normal
Rust functions, captured `Arc` state, or application-owned handler wrappers.

## Error boundaries

Malformed request bytes, size limits, read timeouts, handler errors, and
response writes are distinct boundaries. A handler that wants a structured
client error should convert validation failures to an `HttpResponse`
explicitly:

```rust
use http::StatusCode;
use kode_bridge::HttpResponse;

let response = match ctx.json::<serde_json::Value>() {
    Ok(value) => HttpResponse::json(&value)?,
    Err(error) => HttpResponse::error(StatusCode::BAD_REQUEST, &error.to_string()),
};
```

Do not set a short server `write_timeout` if a handler intentionally performs a
slow local lifecycle operation. Despite the field name, version 0.5 applies it
to handler completion and does not wrap `framed.send(response)` in a separate
write timeout. Slow handlers and slow response readers are therefore different
performance and reliability boundaries.

## Connection ownership

The HTTP server acquires a connection permit before accepting the next client.
Each accepted connection can serve up to
`ServerConfig::max_requests_per_connection` requests. A client pool may open a
replacement connection after that limit; this is expected behavior.

Configure:

- `max_connections` for concurrent open connections;
- `max_requests_per_connection` for keep-alive lifetime;
- `read_timeout` for the next request on an open connection;
- `write_timeout` for handler completion (not the later response write);
- `max_header_size` and `max_request_size` for memory limits.

## Shared state

Handlers are called concurrently. Capture shared state using an `Arc` and an
appropriate synchronization primitive. Do not hold a synchronous lock guard
across `.await`.

```rust
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

let requests = Arc::new(AtomicU64::new(0));
let router = kode_bridge::Router::new().get("/count", {
    let requests = Arc::clone(&requests);
    move |_| {
        let requests = Arc::clone(&requests);
        async move {
            let count = requests.fetch_add(1, Ordering::Relaxed) + 1;
            kode_bridge::HttpResponse::json(&serde_json::json!({"count": count}))
        }
    }
});
```

## Streaming sources

`IpcStreamServer::serve_with_source` owns one `StreamSource`. The source
lifecycle is:

1. `initialize()` before accepting clients;
2. repeated `next_messages()` calls while `has_more()` is true;
3. `cleanup()` when the source loop exits normally.

Cancelling the server task can abort the source task before `cleanup()` runs.
Put mandatory resource cleanup in owned values with `Drop`, not only in the
async source hook.

Messages are sent through a Tokio broadcast channel. When a receiver lags, the
channel reports and skips the missed messages; a write error or write timeout
ends that client connection. Size and write-timeout limits still apply.
`JsonDataSource` is suitable for periodic generation; `IteratorSource` adapts
a Tokio stream of `StreamMessage` values.

The stream server emits raw frames. Do not use `IpcStreamClient` as its direct
peer without an HTTP response layer; the two public types intentionally retain
their pre-0.5 wire behavior.

## Listener lifecycle

On Unix, listener drop removes only the socket path whose device/inode identity
matches the socket created by that listener. Explicit stale overwrite performs
a liveness check and refuses regular files, but the final metadata-check/remove
pair is not atomic. Use a trusted socket directory.

On Windows, each accepted pipe instance is replaced with the next pending
instance before the connected stream is handed upward. The configured SDDL is
reused for every instance. Final dirty writes are flushed independently of
request-future cancellation.

Task cancellation must be awaited before immediate restart so listener drop
and endpoint cleanup have completed.

## Testing server code

The repository's transport integration suite demonstrates reliable readiness
without sleep-based startup guesses:

```bash
cargo test --all-features --test ipc_transport
cargo test --all-features
```

When adding server behavior, cover at least:

- method, header, query, path-parameter, and body handling;
- keep-alive reuse and the per-connection request limit;
- duplicate listener and restart behavior;
- request timeout and cancellation followed by a healthy request;
- 32 concurrent clients on each supported platform;
- Unix mode, stale socket, and cleanup policy;
- Windows SDDL, pipe-busy, final-response flush, and drop delivery.

Run real Linux, macOS, and Windows jobs for transport changes. Cross-compilation
proves type compatibility, not socket or named-pipe runtime behavior.

## Performance work

Use the frozen Criterion suites before changing hot paths:

```bash
cargo bench --all-features --bench bench_version
cargo bench --all-features --bench ipc_transport
```

Keep the same host, toolchain, payloads, sample settings, and endpoint strategy
for before/after comparisons. Inspect confidence intervals and throughput;
single wall-clock runs are not sufficient evidence.
