# Migration and Modernization Guide

Complete guide for migrating to kode-bridge's modern architecture.

## Migrating from 0.4 to 0.5

Version 0.5 replaces the `interprocess` dependency with kode-bridge's Tokio-native transport adapter. The HTTP-over-IPC wire format and the normal client/server constructors, request methods, and permission builders are unchanged. Most applications only need to update the dependency version.

```toml
[dependencies]
kode-bridge = "0.5"
```

The minimum supported Rust version is now 1.87. This matches the toolchain used by kode-bridge CI and the minimum required by its current target dependencies.

Three public type boundaries necessarily change because 0.4 exposed types owned by `interprocess`:

| 0.4 API | 0.5 API | Migration |
|---------|---------|-----------|
| Server `with_listener_options(interprocess::...::ListenerOptions)` | `with_listener_options(kode_bridge::ListenerOptions)` | Import and build `kode_bridge::ListenerOptions`. |
| `ConnectionPool::{new, with_default_config}(Name)` | The same methods accepting `kode_bridge::Endpoint` | Validate the path once with `Endpoint::new(path)?`. |
| `PooledConnection::{stream, into_stream}` returning `LocalSocketStream` | The same methods returning `kode_bridge::IpcStream` | Remove explicit `LocalSocketStream` annotations; `IpcStream` implements Tokio `AsyncRead` and `AsyncWrite`. |

For example, direct pool construction now uses an `Endpoint`:

```rust
use kode_bridge::{
    pool::{ConnectionPool, PoolConfig},
    Endpoint,
};

let endpoint = Endpoint::new("/tmp/service.sock")?;
let pool = ConnectionPool::new(endpoint, PoolConfig::default());
```

Server permission builders keep their prior call shape:

```rust
#[cfg(all(unix, not(target_os = "macos")))]
let server = IpcHttpServer::new("/tmp/service.sock")?
    .with_listener_mode(0o640);

#[cfg(windows)]
let server = IpcHttpServer::new(r"\\.\pipe\service")?
    .with_listener_security_descriptor("D:(A;;GA;;;WD)");
```

Unix listeners still clean up their socket path when dropped. They do not replace stale paths by default. Cleanup checks the path's socket type and device/inode identity, but safe standard-library APIs cannot make compare-and-delete atomic; use a trusted parent directory that untrusted processes cannot modify. Windows endpoint strings must use `\\HOST\pipe\NAME`; malformed paths and invalid SDDL are rejected during configuration.

## 🔄 Migration from Platform-Specific Code

### Migration from Platform-Specific Code

If you were previously using platform-specific imports:

#### Before (Legacy):
```rust
#[cfg(unix)]
use kode_bridge::ipc_cilent::unix::UnixIpcHttpClient as IpcHttpClient;
#[cfg(windows)] 
use kode_bridge::ipc_cilent::windows::WindowsIpcHttpClient as IpcHttpClient;

// Legacy API usage
let client = IpcHttpClient::new("/tmp/service.sock")?;
let response = client.request("GET", "/api/status", None).await?;
```

#### Now (Modern):
```rust
use kode_bridge::{IpcHttpClient, ClientConfig};
use std::time::Duration;

// Modern unified import with advanced configuration
let config = ClientConfig {
    enable_pooling: true,
    max_retries: 3,
    ..Default::default()
};

let client = IpcHttpClient::with_config("/tmp/service.sock", config)?;

// Modern fluent API
let response = client
    .get("/api/status")
    .timeout(Duration::from_secs(10))
    .send()
    .await?;

// Legacy API still supported for backward compatibility
let response = client.request("GET", "/api/status", None).await?;
```

### Modernization Benefits

#### Code Simplification
- **Single Import**: No more platform-specific conditional imports
- **Unified API**: Same interface across all platforms
- **Modern Syntax**: Fluent API similar to reqwest

#### New Features
- **Connection Pooling**: Automatic connection management
- **Type-Safe JSON**: Automatic serialization/deserialization
- **Rich Error Handling**: Categorized errors with context
- **Streaming Support**: Real-time data processing
- **Configuration System**: Flexible environment-based setup

### Migration Checklist

#### Step 1: Update Dependencies
```toml
[dependencies]
kode-bridge = "0.5"
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

#### Step 2: Update Imports
```rust
// Remove platform-specific imports
// #[cfg(unix)] use kode_bridge::ipc_cilent::unix::...

// Add modern unified imports
use kode_bridge::{IpcHttpClient, IpcStreamClient, ClientConfig, StreamClientConfig};
```

#### Step 3: Modernize Client Creation
```rust
// Old way
let client = IpcHttpClient::new(path)?;

// New way with configuration
let client = IpcHttpClient::with_config(path, ClientConfig::default())?;
```

#### Step 4: Update Request Patterns
```rust
// Legacy (still works)
let response = client.request("GET", "/api/data", None).await?;

// Modern fluent API (recommended)
let response = client
    .get("/api/data")
    .timeout(Duration::from_secs(5))
    .send()
    .await?;
```

#### Step 5: Add Error Handling
```rust
// Enhanced error handling
if response.is_success() {
    let data: MyType = response.json()?;
    println!("Success: {:?}", data);
} else if response.is_client_error() {
    println!("Client error: {}", response.status());
} else if response.is_server_error() {
    println!("Server error: {}", response.status());
}
```

## 🎯 Best Practices by Platform

### Unix/Linux/macOS Best Practices

#### Development
```bash
# Use /tmp for development
CUSTOM_SOCK=/tmp/dev_myapp.sock

# Ensure cleanup
trap "rm -f /tmp/dev_myapp.sock" EXIT
```

#### Production
```bash
# Use /var/run for production
CUSTOM_SOCK=/var/run/myapp/api.sock

# Set proper permissions
sudo mkdir -p /var/run/myapp
sudo chown myapp:myapp /var/run/myapp
```

### Windows Best Practices

#### Development
```cmd
REM Use descriptive pipe names
set CUSTOM_PIPE=\\\\.\\pipe\\myapp_dev
```

#### Production
```cmd
REM Use service-specific names
set CUSTOM_PIPE=\\\\.\\pipe\\myapp_production

REM For Windows services
net start MyAppService
```

#### PowerShell
```powershell
# Environment setup
$env:CUSTOM_PIPE="\\\\.\\pipe\\myapp"

# Service management
Start-Service -Name "MyAppService"
```

## 🚀 Performance Optimization Tips

### Connection Pooling
```rust
// Optimize pool settings based on usage
let config = ClientConfig {
    enable_pooling: true,
    pool_max_size: 20,      // Adjust based on concurrent load
    pool_min_idle: 5,       // Keep minimum connections ready
    pool_max_idle_time_ms: 30000, // 30 seconds
    ..Default::default()
};
```

### Buffer Optimization
```rust
// For high-throughput streaming
let stream_config = StreamClientConfig {
    buffer_size: 65536, // 64KB for high-volume data
    ..Default::default()
};

// For low-latency scenarios
let stream_config = StreamClientConfig {
    buffer_size: 4096,  // 4KB for low latency
    ..Default::default()
};
```

### Timeout Configuration
```rust
// Different timeout strategies
let config = ClientConfig {
    default_timeout: Duration::from_secs(30), // Global timeout
    ..Default::default()
};

// Per-request timeout
let response = client
    .get("/api/data")
    .timeout(Duration::from_secs(5)) // Override global timeout
    .send()
    .await?;
```

## 📈 Performance Comparison

### Before vs After Migration

| Feature | Legacy | Modern | Improvement |
|---------|--------|--------|-------------|
| Code Lines | 50+ lines | 10-15 lines | 70% reduction |
| Error Handling | Basic | Rich categorized | Better debugging |
| Connection Management | Manual | Automatic pooling | 3x throughput |
| Platform Support | Conditional | Unified | Simplified code |
| API Style | Verbose | Fluent/Chainable | Better readability |
| Type Safety | Limited | Full JSON support | Fewer runtime errors |

### Migration Timeline

1. **Phase 1 (Week 1)**: Update dependencies and imports
2. **Phase 2 (Week 2)**: Modernize core request patterns
3. **Phase 3 (Week 3)**: Add advanced features (pooling, streaming)
4. **Phase 4 (Week 4)**: Performance optimization and testing
5. **Phase 5 (Week 5)**: Production deployment and monitoring
