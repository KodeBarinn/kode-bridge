# Quick Start Guide

kode-bridge provides HTTP-style request/response and streaming APIs over Unix
domain sockets on Unix platforms and named pipes on Windows. Version 0.5
requires Rust 1.87 or newer.

## Add the dependency

```toml
[dependencies]
# Client only (default)
kode-bridge = "0.5"

# Both client and server
kode-bridge = { version = "0.5", features = ["full"] }

tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde_json = "1"
```

Available crate features:

- `client` (default): HTTP and streaming clients
- `server`: HTTP and streaming servers
- `full`: both `client` and `server`

## Choose an endpoint

```rust
#[cfg(unix)]
const IPC_ENDPOINT: &str = "/tmp/my-service.sock";

#[cfg(windows)]
const IPC_ENDPOINT: &str = r"\\.\pipe\my-service";
```

The library validates the endpoint but does not read `CUSTOM_SOCK` or
`CUSTOM_PIPE` itself. Those environment variables are conventions used by the
repository examples.

## HTTP client

```rust
use kode_bridge::{IpcHttpClient, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    let endpoint = "/tmp/my-service.sock";
    #[cfg(windows)]
    let endpoint = r"\\.\pipe\my-service";

    let client = IpcHttpClient::new(endpoint)?;
    let response = client
        .get("/api/version")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("status={} body={}", response.status(), response.body()?);
    Ok(())
}
```

The fluent client supports GET, POST, PUT, DELETE, PATCH, HEAD, and OPTIONS,
headers, JSON bodies, per-request timeouts, pooling, and typed JSON responses.

## HTTP server

Enable `server` or `full`, then create a router:

```rust
use kode_bridge::{HttpResponse, IpcHttpServer, Result, Router};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    let endpoint = "/tmp/my-service.sock";
    #[cfg(windows)]
    let endpoint = r"\\.\pipe\my-service";

    let router = Router::new()
        .get("/health", |_| async {
            HttpResponse::json(&json!({"status": "healthy"}))
        })
        .post("/echo", |ctx| async move {
            HttpResponse::json(&ctx.json::<serde_json::Value>()?)
        });

    let mut server = IpcHttpServer::new(endpoint)?.router(router);
    server.serve().await
}
```

Run the checked-in server examples with:

```bash
cargo run --features server --example http_server
cargo run --features server --example stream_server
```

## Client examples

The examples read `CUSTOM_SOCK` on Unix and `CUSTOM_PIPE` on Windows:

```bash
# Unix
CUSTOM_SOCK=/tmp/my-service.sock cargo run --example request

# Windows PowerShell
$env:CUSTOM_PIPE='\\.\pipe\my-service'
cargo run --example request
```

Other useful examples include `request_large`, `elegant_http`,
`elegant_stream`, `traffic`, `traffic_monitor`, and `two_clients`.

## Streaming APIs

- `IpcStreamClient` sends an HTTP-style request and processes a streamed HTTP
  response line by line or as JSON values.
- `IpcStreamServer` broadcasts raw newline-delimited `StreamMessage` frames to
  connected IPC clients.

They use different wire shapes in 0.5 and should not be assumed to connect
directly to one another. See the checked-in examples and
[Server Guide](../SERVER_GUIDE.md) for the current server API.

## Run checks and benchmarks

```bash
cargo test --all-features
cargo doc --all-features --no-deps

cargo bench --all-features --bench bench_version
cargo bench --all-features --bench ipc_transport
```

Criterion writes HTML reports below `target/criterion/`. Benchmark results are
host- and platform-specific; use the same machine, toolchain, build mode, and
parameters for before/after comparisons.

For a 0.4 upgrade, continue with the
[0.4 to 0.5 Migration Guide](./migration-guide.md).
