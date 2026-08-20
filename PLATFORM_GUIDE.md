# kode-bridge Platform Guide

This guide describes the platform contract for kode-bridge 0.5. The crate uses
Tokio Unix domain sockets on Unix platforms and Tokio named pipes on Windows.
The HTTP-over-IPC protocol, pooling, routing, retries, and streaming logic are
implemented by kode-bridge above that transport layer.

## Documentation map

- [Quick Start](./docs/quick-start.md)
- [Platform Configuration](./docs/platform-configuration.md)
- [0.4 to 0.5 Migration](./docs/migration-guide.md)
- [Server Guide](./SERVER_GUIDE.md)
- [Server Development Notes](./docs/server-development.md)
- [Runnable examples](./examples/)
- [API documentation](https://docs.rs/kode-bridge)

## Supported platforms

| Platform | Transport | Endpoint example | Server feature |
| --- | --- | --- | --- |
| Linux and other supported Unix systems | Unix domain socket | `/run/my-app/service.sock` | `server` |
| macOS | Unix domain socket | `/tmp/my-app.sock` | `server` |
| Windows | Local named pipe | `\\.\pipe\my-app` | `server` |

Rust 1.87 is the minimum supported toolchain for version 0.5.

## Feature flags

```toml
[dependencies]
# Client only (default)
kode-bridge = "0.5"

# Server only
kode-bridge = { version = "0.5", features = ["server"] }

# Client and server
kode-bridge = { version = "0.5", features = ["full"] }
```

The `client` and `server` features control the high-level APIs. The transport
implementation is selected at compile time with `cfg(unix)` or `cfg(windows)`;
there is no runtime platform switch or alternate runtime backend.

## Endpoint selection

Use conditional compilation when one program targets multiple platforms:

```rust
#[cfg(unix)]
let endpoint = "/tmp/service.sock";

#[cfg(windows)]
let endpoint = r"\\.\pipe\service";

let client = kode_bridge::IpcHttpClient::new(endpoint)?;
```

Endpoint rules:

- Unix endpoints are file-system paths. Interior NUL bytes are rejected.
- Windows endpoints must have the form `\\HOST\pipe\NAME`. Use `.` as the
  host for local pipes: `\\.\pipe\NAME`.
- Windows servers reject remote clients, so a remote host component is not a
  supported deployment mode.
- Validation happens in `Endpoint::new` and in the high-level client/server
  constructors.

kode-bridge does not automatically read environment variables. The repository
examples use `CUSTOM_SOCK` and `CUSTOM_PIPE` as application-level conventions.

## Unix listener behavior

The default listener:

- creates a Tokio `UnixListener`;
- does not overwrite an existing socket path;
- removes its own socket path when dropped;
- checks socket type and device/inode identity before cleanup.

For an explicit stale-socket policy:

```rust
#[cfg(unix)]
let options = kode_bridge::ListenerOptions::new()
    .reclaim_name(true)
    .try_overwrite(true)
    .max_spin_time(std::time::Duration::from_millis(100));

#[cfg(unix)]
let server = kode_bridge::IpcHttpServer::new("/run/my-app/service.sock")?
    .with_listener_options(options);
```

`try_overwrite(true)` only removes a path that is still the same stale Unix
socket after the liveness and identity checks. It refuses normal files and
live listeners. The final metadata check and file removal cannot be one atomic
standard-library operation, so use a parent directory that untrusted users
cannot modify.

Custom listener mode is applied before listening on supported Unix platforms:

```rust
#[cfg(all(unix, not(target_os = "macos")))]
let server = kode_bridge::IpcHttpServer::new("/run/my-app/service.sock")?
    .with_listener_mode(0o660);
```

Custom mode currently returns `Unsupported` on macOS. Directory ownership and
permissions remain the primary production access boundary.

## Windows listener behavior

Windows uses Tokio named pipes. The listener keeps a pending next instance so
concurrent connects do not encounter a handoff gap. Clients retry
`ERROR_PIPE_BUSY` asynchronously. High-level HTTP and streaming requests wrap
connection setup in their request timeout. Direct `IpcStream::connect` and
pool-preheat operations need an application-owned cancellation boundary.

Named pipes reject remote clients. To set a local security descriptor:

```rust
#[cfg(windows)]
let server = kode_bridge::IpcHttpServer::new(r"\\.\pipe\service")?
    .with_listener_security_descriptor("D:(A;;GA;;;WD)");
```

The descriptor is applied to every pipe instance. Invalid SDDL retains the 0.4
API behavior and panics during builder configuration. Do not pass unvalidated
user input as SDDL.

The backend uses 512-byte inbound and outbound pipe buffers and preserves
flush-on-drop delivery for a final response. These are compatibility details,
not public tuning knobs.

## Containers and services

For Unix containers, mount a dedicated socket directory and set its ownership
and mode explicitly. Do not put a privileged service socket in a world-writable
directory.

Windows container named-pipe access depends on the container host and service
configuration. kode-bridge validates and opens the pipe but does not configure
container isolation or Windows service identities.

## Performance evidence

Run both benchmark suites with the required features:

```bash
cargo bench --all-features --bench bench_version
cargo bench --all-features --bench ipc_transport
```

`bench_version` covers builders, codec/router work, and in-memory duplex paths.
`ipc_transport` covers real cold connections, direct and pooled requests,
payload sizes, and a concurrent burst. Compare results only on the same host,
platform, toolchain, build mode, and benchmark parameters. No fixed connection
reuse percentage or cross-platform performance ratio is guaranteed.

## Platform acceptance

Compilation for a target is not runtime acceptance. Changes to transport,
permissions, or cleanup should be exercised on real Linux, macOS, and Windows
runners. A normal GitHub Windows runner does not prove communication between a
privileged service and a standard-user client; validate that identity boundary
in the intended deployment environment.
