# Platform Configuration Guide

kode-bridge constructors take an endpoint path directly. Environment variables
are an application concern; the checked-in examples use `CUSTOM_SOCK` on Unix
and `CUSTOM_PIPE` on Windows, but the library does not read either variable.

## Endpoint configuration

### Unix

```bash
CUSTOM_SOCK=/tmp/my-service.sock cargo run --example request
```

Use `/tmp` for local development. For a production service, prefer a dedicated
directory such as `/run/my-app` whose ownership and permissions prevent
untrusted path replacement.

```bash
sudo install -d -o myapp -g myapp -m 0750 /run/my-app
CUSTOM_SOCK=/run/my-app/api.sock cargo run --example request
```

Unix endpoints may be relative or absolute file-system paths, but cannot
contain an interior NUL. The parent directory must already exist.

### Windows

Command Prompt:

```cmd
set CUSTOM_PIPE=\\.\pipe\my-service
cargo run --example request
```

PowerShell:

```powershell
$env:CUSTOM_PIPE='\\.\pipe\my-service'
cargo run --example request
```

Windows endpoints must use `\\HOST\pipe\NAME`. kode-bridge servers reject
remote clients, so use the local `\\.\pipe\NAME` form.

### `.env` examples

Unquoted `.env` values do not need Rust string-literal escaping:

```env
# Unix
CUSTOM_SOCK=/tmp/my-service.sock

# Windows
CUSTOM_PIPE=\\.\pipe\my-service
```

The examples call `dotenvy::dotenv()` before reading these values. Add
`dotenvy` to your own application if you want the same behavior.

## HTTP client configuration

`ClientConfig` contains a nested `PoolConfig`; the older flattened
`pool_max_size` and `pool_min_idle` fields do not exist.

```rust
use kode_bridge::{pool::PoolConfig, ClientConfig, IpcHttpClient};
use std::time::Duration;

let pool = PoolConfig {
    max_size: 16,
    min_idle: 4,
    max_idle_time_ms: 30_000,
    connection_timeout_ms: 5_000,
    retry_delay_ms: 25,
    max_retries: 3,
    max_concurrent_requests: 16,
    max_requests_per_second: None,
};

let config = ClientConfig {
    default_timeout: Duration::from_secs(5),
    pool_config: pool,
    enable_pooling: true,
    max_retries: 3,
    retry_delay: Duration::from_millis(25),
    max_concurrent_requests: 16,
    max_requests_per_second: Some(50.0),
    require_windows_server_system: false,
    #[cfg(windows)]
    windows_server_pid_verifier: None,
};

let client = IpcHttpClient::with_config(endpoint, config)?;
```

Choose pool limits from measured concurrent demand. A larger pool consumes more
open handles and memory and does not guarantee lower latency.

On Windows, set `require_windows_server_system` when any LocalSystem pipe
server is acceptable. Set `windows_server_pid_verifier` when the application
must compare the PID reported by the connected pipe handle with an external
service policy such as SCM status. Both checks run before request bytes are
sent, once per physical connection; pooled reuse does not rerun them.

## Streaming client configuration

```rust
use kode_bridge::{IpcStreamClient, StreamClientConfig};
use std::time::Duration;

let config = StreamClientConfig {
    default_timeout: Duration::from_secs(60),
    max_retries: 3,
    retry_delay: Duration::from_millis(100),
    buffer_size: 16 * 1024,
};

let client = IpcStreamClient::with_config(endpoint, config)?;
```

The buffer size is a user-space read buffer, not the Windows named-pipe kernel
buffer size. Measure memory and throughput before increasing it broadly.

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

`read_timeout` bounds waiting for and parsing the next request. Despite its
name, `write_timeout` currently bounds the handler future; the subsequent
response send does not have a separate server-side timeout in version 0.5.

## Listener options

### Unix cleanup, overwrite, and mode

```rust
#[cfg(unix)]
let options = kode_bridge::ListenerOptions::new()
    .reclaim_name(true)
    .try_overwrite(true)
    .max_spin_time(std::time::Duration::from_millis(100));

#[cfg(unix)]
let options = options.mode(0o660);
```

- `reclaim_name(true)` is the default.
- `try_overwrite(false)` is the default.
- Explicit overwrite refuses non-socket paths and live listeners.
- Custom mode is applied after bind and before listen on Linux and macOS.
- Use a trusted parent directory because metadata-check then remove is not an
  atomic compare-and-delete operation.

### Windows SDDL

```rust
#[cfg(windows)]
let options = kode_bridge::ListenerOptions::new()
    .security_descriptor("D:(A;;GA;;;WD)");
```

The descriptor is parsed when configuring the builder and applied to every
pipe instance. Invalid SDDL panics for compatibility with the 0.4 API. Prefer a
least-privilege descriptor; `WD` grants access to Everyone and should not be a
copy-paste production default.

## Operational checks

- Confirm the client and server resolve exactly the same endpoint string.
- Ensure the Unix socket parent exists and has the intended ownership.
- Treat `Address already in use` as a live-listener or stale-path decision;
  enable overwrite only when that policy is intentional.
- On Windows, distinguish an absent pipe from `ERROR_PIPE_BUSY`; kode-bridge
  waits asynchronously for busy instances. High-level requests apply their
  request timeout, while direct stream connects and preheating need an
  application-owned cancellation boundary.
- Exercise service identity and ACL behavior with the actual privileged and
  unprivileged accounts used in production.

## Benchmark configuration

```bash
cargo bench --all-features --bench bench_version
cargo bench --all-features --bench ipc_transport
```

Criterion results are machine-specific. Keep the endpoint generator, payload,
sample parameters, toolchain, and host unchanged when comparing versions.
