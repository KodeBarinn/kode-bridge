# Migrating from 0.4 to 0.5

kode-bridge 0.5 replaces `interprocess` with a small transport layer backed by
Tokio Unix domain sockets and Windows named pipes. The HTTP-over-IPC wire
format, normal client and server constructors, request builders, routing, pool
behavior, and streaming formats are unchanged.

## Requirements

Update the dependency and toolchain together:

```toml
[dependencies]
kode-bridge = "0.5"
```

- The minimum supported Rust version is 1.87.
- The default feature remains `client`.
- Use `features = ["server"]` for server-only applications and
  `features = ["full"]` when both sides are needed.

## Necessary API changes

Version 0.4 exposed three types owned by `interprocess`. Version 0.5 replaces
only those boundaries with kode-bridge-owned types.

| 0.4 API | 0.5 API | Migration |
| --- | --- | --- |
| `with_listener_options(interprocess::...::ListenerOptions)` | `with_listener_options(kode_bridge::ListenerOptions)` | Import and construct `kode_bridge::ListenerOptions`. |
| `ConnectionPool::{new, with_default_config}(Name)` | The same methods accepting `kode_bridge::Endpoint` | Validate the path once with `Endpoint::new(path)?`. |
| `PooledConnection::{stream, into_stream}` returning `LocalSocketStream` | The same methods returning `kode_bridge::IpcStream` | Remove explicit `LocalSocketStream` annotations. `IpcStream` implements Tokio `AsyncRead` and `AsyncWrite`, including vectored writes. |

Direct pool construction now uses an `Endpoint`:

```rust
use kode_bridge::{
    pool::{ConnectionPool, PoolConfig},
    Endpoint,
};

let endpoint = Endpoint::new("/tmp/service.sock")?;
let pool = ConnectionPool::new(endpoint, PoolConfig::default());
```

Most applications only use `IpcHttpClient::new`, `IpcStreamClient::new`,
`IpcHttpServer::new`, or `IpcStreamServer::new`; those constructors still
accept a path and do not require an explicit `Endpoint`.

## Endpoint rules

Use a platform-appropriate endpoint:

```rust
#[cfg(unix)]
let endpoint = "/tmp/service.sock";

#[cfg(windows)]
let endpoint = r"\\.\pipe\service";
```

- Unix endpoints are file-system paths and cannot contain an interior NUL.
- Windows endpoints must use `\\HOST\pipe\NAME`; normal local endpoints use
  `\\.\pipe\NAME`.
- Endpoint validation now happens when the client, server, or `Endpoint` is
  constructed.

## Listener configuration

The convenience builders retain their 0.4 call shape:

```rust
#[cfg(all(unix, not(target_os = "macos")))]
let server = IpcHttpServer::new("/tmp/service.sock")?
    .with_listener_mode(0o640);

#[cfg(windows)]
let server = IpcHttpServer::new(r"\\.\pipe\service")?
    .with_listener_security_descriptor("D:(A;;GA;;;WD)");
```

For advanced Unix listener behavior:

```rust
#[cfg(unix)]
let options = kode_bridge::ListenerOptions::new()
    .reclaim_name(true)
    .try_overwrite(true)
    .max_spin_time(std::time::Duration::from_millis(100));
```

Important behavior:

- `reclaim_name(true)` remains the default and removes the listener's own Unix
  socket path on drop.
- Stale Unix sockets are not replaced unless `try_overwrite(true)` is set.
- Overwrite refuses regular files and listeners that still accept connections.
- Unix compare-and-delete cannot be made atomic with the safe standard-library
  APIs used here. Put production sockets in a trusted parent directory.
- Custom Unix mode is applied before listening on supported Unix platforms.
  It currently returns `Unsupported` on macOS.
- Windows named pipes reject remote clients. A configured SDDL descriptor is
  applied to every pipe instance.
- Invalid Windows SDDL retains the 0.4 builder behavior and panics during
  configuration. Validate untrusted SDDL before passing it to the builder.

## Behavior preserved by the new transport

- HTTP request and response bytes, headers, methods, and JSON handling
- Client request timeouts and retry boundaries
- Pool reuse, invalidation, preheating, and per-connection request limits
- Unix listener cleanup and explicit stale-path policy
- Windows pipe-busy retry, 512-byte pipe buffers, and flush-on-drop delivery
- HTTP-style streaming client parsing and newline-delimited stream-server frames

`IpcStreamClient` and `IpcStreamServer` are not a matched pair in 0.5:
`IpcStreamClient` consumes an HTTP-style streaming response, while
`IpcStreamServer` broadcasts raw newline-delimited messages. This is existing
behavior, not a transport migration change.

## Verification checklist

After updating:

```bash
cargo check --all-features
cargo test --all-features
cargo doc --all-features --no-deps
```

Applications that directly used one of the three replaced public types should
also compile their own public API or downstream fixture.

To compare performance on the same host and toolchain:

```bash
cargo bench --all-features --bench bench_version
cargo bench --all-features --bench ipc_transport
```

Do not compare Criterion results from different hosts or shared CI runners as
if they were a controlled before/after measurement.
