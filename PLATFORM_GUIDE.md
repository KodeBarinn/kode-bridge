# kode-bridge Platform Guide

**Complete cross-platform usage guide for HTTP Over IPC with client and server capabilities**

This guide provides comprehensive information for using kode-bridge across Unix/Linux/macOS and Windows systems, including both client and server functionality.

## 📚 Documentation Structure

This platform guide is organized into focused sections for easier navigation:

### Quick References
- **[Quick Start Guide](./docs/quick-start.md)** - Get up and running quickly with examples
- **[Platform Configuration](./docs/platform-configuration.md)** - Environment setup and configuration
- **[Migration Guide](./docs/migration-guide.md)** - Modernization and best practices
- **[Server Development](./docs/server-development.md)** - Complete server development guide

### Additional Resources
- **[Server Guide](./SERVER_GUIDE.md)** - Comprehensive server implementation guide
- **[Examples Directory](./examples/)** - Complete runnable examples
- **[API Documentation](https://docs.rs/kode-bridge)** - Full API reference

## 🚀 Overview

kode-bridge provides:

### Client Capabilities (Default Feature)
- **`IpcHttpClient`**: HTTP-style request/response with fluent API
- **`IpcStreamClient`**: Real-time streaming for continuous data monitoring
- **Cross-platform compatibility**: Automatic platform detection
- **Connection pooling**: Optimized connection management
- **Type-safe JSON**: Automatic serialization/deserialization

### Server Capabilities (Server Feature)
- **`IpcHttpServer`**: HTTP server with Express.js-style routing
- **`IpcStreamServer`**: Real-time streaming server with broadcasting
- **Middleware support**: Request/response interceptors
- **Multi-client management**: Connection lifecycle and cleanup
- **Feature-gated compilation**: Include only what you need

## 🌍 Cross-Platform Support

### Automatic Platform Detection
```rust
// Same code works on all platforms
let client = IpcHttpClient::new("/tmp/service.sock")?;  // Unix
let client = IpcHttpClient::new(r"\\.\pipe\service")?; // Windows
```

### Platform-Specific Optimizations
- **Unix/Linux/macOS**: Unix Domain Sockets with superior performance
- **Windows**: Named Pipes with native Windows integration
- **Feature flags**: Compile only client or server functionality as needed

## 🔧 Feature Flags

Configure your `Cargo.toml` based on your needs:

```toml
[dependencies]
# Client only (default) - 🔥 Most common setup
kode-bridge = "0.1"

# Server only - For services that only serve data
kode-bridge = { version = "0.1", features = ["server"] }

# Both client and server - Full-featured applications
kode-bridge = { version = "0.1", features = ["full"] }
```

## 📋 Quick Examples

### Client Example
```rust
use kode_bridge::IpcHttpClient;
use std::time::Duration;

let client = IpcHttpClient::new("/tmp/service.sock")?;

// Modern fluent API
let response = client
    .get("/api/status")
    .timeout(Duration::from_secs(5))
    .send()
    .await?;

println!("Status: {}", response.status());
```

### Server Example  
```rust
use kode_bridge::{IpcHttpServer, Router, HttpResponse};
use serde_json::json;

let router = Router::new()
    .get("/health", |_| async move {
        HttpResponse::json(&json!({"status": "healthy"}))
    });

let mut server = IpcHttpServer::new("/tmp/server.sock")?
    .router(router);

server.serve().await?;
```

## 🎯 Use Cases

### Client Applications
- **Local service communication**: Connect to Clash, Mihomo, proxy services
- **Real-time monitoring**: Stream traffic data, logs, metrics
- **System integration**: Replace REST API calls with high-performance IPC
- **Configuration management**: Dynamic config updates with immediate feedback

### Server Applications  
- **API services**: HTTP-style endpoints over IPC
- **Real-time data providers**: Stream metrics, events, logs
- **System bridges**: Connect different applications via IPC
- **Service orchestration**: Coordinate multiple local services

## 📊 Performance

### Benchmarks
```bash
# Run comprehensive benchmarks
cargo bench

# View results
open target/criterion/report/index.html
```

### Performance Characteristics
- **Connection pooling**: 95%+ connection reuse on Unix, 90%+ on Windows
- **Low latency**: Sub-millisecond response times for local IPC
- **High throughput**: Optimized for both request/response and streaming
- **Memory efficient**: Automatic backpressure and client management

## 🔗 Integration Patterns

### Environment-Based Configuration
```rust
// Automatically adapts to platform
#[cfg(unix)]
let path = env::var("CUSTOM_SOCK").unwrap_or("/tmp/service.sock".to_string());
#[cfg(windows)]  
let path = env::var("CUSTOM_PIPE").unwrap_or(r"\\.\pipe\service".to_string());
```

### Docker and Containerization
```dockerfile
# Unix containers
VOLUME ["/tmp/ipc"]
ENV CUSTOM_SOCK=/tmp/ipc/service.sock

# Windows containers  
ENV CUSTOM_PIPE=\\.\pipe\service
```

### Service Discovery
```rust
// Dynamic service discovery
let services = vec![
    "/tmp/service1.sock",
    "/tmp/service2.sock", 
    "/tmp/service3.sock"
];

for service_path in services {
    if let Ok(client) = IpcHttpClient::new(service_path) {
        // Use first available service
        break;
    }
}
```

## 🚀 Getting Started

1. **Read the [Quick Start Guide](./docs/quick-start.md)** for immediate setup
2. **Configure your platform** using the [Platform Configuration](./docs/platform-configuration.md) guide
3. **Explore examples** in the [examples/](./examples/) directory
4. **For server development**, see the [Server Development](./docs/server-development.md) guide
5. **Migrating existing code?** Check the [Migration Guide](./docs/migration-guide.md)

## 🤝 Community and Support

- **[GitHub Issues](https://github.com/KodeBarinn/kode-bridge/issues)** - Bug reports and feature requests
- **[GitHub Discussions](https://github.com/KodeBarinn/kode-bridge/discussions)** - Community help and discussions
- **[Crates.io](https://crates.io/crates/kode-bridge)** - Latest releases and documentation

---

**kode-bridge** - Modern HTTP Over IPC for Rust 🚀