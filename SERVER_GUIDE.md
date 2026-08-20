# kode-bridge Server Guide

This guide covers the server APIs available in kode-bridge 0.5. Enable the
`server` feature for server-only programs or `full` when the same crate also
uses client APIs.

```toml
[dependencies]
kode-bridge = { version = "0.5", features = ["server"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
serde_json = "1"
http = "1"
```

Rust 1.87 or newer is required.

## HTTP server

`IpcHttpServer` parses HTTP-style requests from an IPC connection, routes them
by method and path, and encodes an HTTP-style response on the same connection.

```rust
use http::StatusCode;
use kode_bridge::{HttpResponse, IpcHttpServer, Result, Router};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    let endpoint = "/tmp/kode-bridge-server.sock";
    #[cfg(windows)]
    let endpoint = r"\\.\pipe\kode-bridge-server";

    let router = Router::new()
        .get("/health", |_| async {
            HttpResponse::json(&json!({"status": "healthy"}))
        })
        .get("/users/:id", |ctx| async move {
            let id = ctx
                .path_params
                .get("id")
                .cloned()
                .unwrap_or_default();
            HttpResponse::json(&json!({"id": id}))
        })
        .post("/echo", |ctx| async move {
            match ctx.json::<serde_json::Value>() {
                Ok(value) => HttpResponse::json(&value),
                Err(error) => Ok(HttpResponse::error(
                    StatusCode::BAD_REQUEST,
                    &error.to_string(),
                )),
            }
        });

    let mut server = IpcHttpServer::new(endpoint)?.router(router);
    server.serve().await
}
```

The router supports GET, POST, PUT, and DELETE helpers. Use `add_route` with an
`http::Method` for another method. Route parameters use `:name`, for example
`/users/:id`.

The current router does not provide middleware or static-file helpers. Put
cross-cutting behavior in handler functions or an application-owned wrapper.

## Request and response APIs

`RequestContext` exposes:

- `method`, `uri`, `headers`, and raw `body` fields;
- `json::<T>()` and `text()` body parsing;
- `query_params()` for decoded query pairs;
- `path_params` and `path_params()` for route captures;
- connection ID and timing through `client_info` and `timestamp`.

Handlers return `kode_bridge::Result<HttpResponse>`. Responses can be built
with `HttpResponse::json`, `HttpResponse::text`, `HttpResponse::error`, or the
response builder:

```rust
let response = HttpResponse::builder()
    .status(http::StatusCode::CREATED)
    .header("content-type", "application/json")
    .json(&serde_json::json!({"created": true}))?
    .build();
```

## HTTP server configuration

```rust
use kode_bridge::ServerConfig;
use std::time::Duration;

let config = ServerConfig {
    max_connections: 128,
    read_timeout: Duration::from_secs(5),
    write_timeout: Duration::from_secs(5),
    max_request_size: 10 * 1024 * 1024,
    max_header_size: 4096,
    enable_logging: true,
    max_requests_per_connection: 32,
    shutdown_timeout: Duration::from_secs(3),
};
```

Create the server with `IpcHttpServer::with_config(endpoint, config)`.

- `max_connections` bounds concurrently accepted connections.
- `read_timeout` bounds waiting for and parsing the next request.
- Despite its name, `write_timeout` currently bounds the handler future. The
  subsequent `framed.send(response)` is not wrapped in a separate timeout.
- `max_requests_per_connection` bounds keep-alive reuse on one connection.
- Size limits should be set from the largest supported request, with overhead.
- `enable_logging` controls per-request server logging.

`ServerStats` reports connections, requests, responses, errors, and start time.
The server does not currently expose a cloneable runtime stats/control handle;
plan ownership before moving the server into a task.

## Streaming server

`IpcStreamServer` broadcasts raw `StreamMessage` frames. JSON and text messages
are newline-delimited; binary messages are written as provided. A periodic
`JsonDataSource` is the shortest supported setup:

```rust
use kode_bridge::{IpcStreamServer, JsonDataSource, Result, StreamServerConfig};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    let endpoint = "/tmp/kode-bridge-stream.sock";
    #[cfg(windows)]
    let endpoint = r"\\.\pipe\kode-bridge-stream";

    let config = StreamServerConfig {
        max_connections: 64,
        buffer_size: 64 * 1024,
        write_timeout: Duration::from_secs(5),
        max_message_size: 1024 * 1024,
        enable_logging: true,
        shutdown_timeout: Duration::from_secs(5),
        broadcast_capacity: 1000,
        keepalive_interval: Duration::from_secs(30),
    };

    let source = JsonDataSource::new(
        || Ok(json!({"status": "running"})),
        Duration::from_secs(1),
    );

    let mut server = IpcStreamServer::with_config(endpoint, config)?;
    server.serve_with_source(source).await
}
```

For application-defined production logic, implement `StreamSource` directly.
It supplies `next_messages`, `has_more`, `initialize`, and `cleanup`; see
[stream_server.rs](./examples/stream_server.rs) and the API documentation for
the exact signatures.

`IpcStreamClient` is not the direct client for `IpcStreamServer` in 0.5. The
former parses an HTTP-style streamed response, while the latter emits raw
newline-delimited frames. Use a raw Tokio `AsyncRead` client for the stream
server protocol, or expose an HTTP-style streaming handler for
`IpcStreamClient`.

The public `broadcast()` method requires the server's broadcast channel to be
initialized, but `serve(&mut self)` holds the mutable server borrow. Version
0.5 does not expose a cloneable broadcast/control handle, so
`serve_with_source` is the practical public API for ongoing broadcasts.

## Listener permissions and cleanup

### Unix

```rust
#[cfg(all(unix, not(target_os = "macos")))]
let server = IpcHttpServer::new("/run/my-app/service.sock")?
    .with_listener_mode(0o660);
```

The default listener removes its own socket path when dropped. It does not
overwrite a stale path unless `ListenerOptions::try_overwrite(true)` is set.
Use a trusted parent directory. Custom mode is currently unsupported on macOS.

### Windows

```rust
#[cfg(windows)]
let server = IpcHttpServer::new(r"\\.\pipe\my-app")?
    .with_listener_security_descriptor("D:(A;;GA;;;WD)");
```

Windows named pipes reject remote clients. SDDL is applied to every pipe
instance; invalid SDDL panics during builder configuration for 0.4 API
compatibility. Use a least-privilege descriptor for production services.

## Task lifecycle

`serve()` runs until it is cancelled or its internal shutdown channel fires;
accept errors are logged and the accept loop continues. Version 0.5 does not
expose a separate shutdown handle. When the application owns the server task,
cancellation is the usable external shutdown mechanism:

```rust
let task = tokio::spawn(async move { server.serve().await });
tokio::signal::ctrl_c().await?;
task.abort();
let _ = task.await;
```

Await the cancelled task so the listener is dropped before restarting on the
same endpoint. On Unix, that drop performs identity-checked path cleanup.

## Performance guidance

- Reuse pooled client connections instead of increasing server limits first.
- Set `max_connections` and broadcast capacity from measured concurrency and
  per-connection memory.
- Keep request and message limits explicit.
- Do not shorten `write_timeout` below the slowest supported handler. Treat a
  response-write deadline as an open server limitation in version 0.5.
- Run `cargo bench --all-features --bench ipc_transport` for transport changes.
- Treat shared CI runners as functional evidence, not stable performance data.

Complete runnable programs are maintained in
[http_server.rs](./examples/http_server.rs),
[stream_server.rs](./examples/stream_server.rs), and
[server.rs](./examples/server.rs).
