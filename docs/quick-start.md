# Quick Start Guide

This guide helps you get started with kode-bridge quickly across all platforms.

## 🚀 Quick Start Examples

### Modern API Examples
```bash
# 🌟 Essential Examples
cargo run --example request              # Basic HTTP requests with modern API
cargo run --example request_large        # Large data handling and optimization
cargo run --example modern_example       # Complete modern API overview

# 🎨 Advanced Examples
cargo run --example elegant_http         # Full HTTP client feature demonstration
cargo run --example elegant_stream       # Complete streaming client showcase
cargo run --example two_clients          # Architecture comparison and best practices
cargo run --example traffic              # Professional traffic monitoring implementation
```

### Server Examples
```bash
# HTTP server examples (requires server feature)
cargo run --example http_server --features server

# Streaming server examples (requires server feature)
cargo run --example stream_server --features server
```

### Legacy Compatibility
```bash
# All examples support both modern fluent API and legacy methods
# for seamless migration and backward compatibility
```

## 📊 Performance Benchmarks

### Running Benchmarks
```bash
# Run all performance benchmarks
cargo bench

# View detailed benchmark reports
open target/criterion/report/index.html  # macOS/Linux
start target/criterion/report/index.html # Windows
```

### Benchmark Features
- **Cross-platform optimization**: Automatic platform-specific tuning
- **Connection pooling performance**: Demonstrates pooling benefits
- **Streaming vs Request/Response**: Performance comparison
- **Error handling overhead**: Modern error system benchmarks

## 🔧 Basic Setup

### Dependencies
Add to your `Cargo.toml`:

```toml
[dependencies]
# Client only (default)
kode-bridge = "0.1"

# Server only  
kode-bridge = { version = "0.1", features = ["server"] }

# Both client and server
kode-bridge = { version = "0.1", features = ["full"] }

# Required runtime
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### Available Features
- **`client`** (default) - HTTP and streaming client functionality
- **`server`** - HTTP and streaming server functionality  
- **`full`** - Both client and server capabilities

### Basic Client Usage
```rust
use kode_bridge::{IpcHttpClient, IpcStreamClient};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Automatically detect platform and use appropriate IPC path
    #[cfg(unix)]
    let ipc_path = "/tmp/my_service.sock";
    #[cfg(windows)]
    let ipc_path = r"\\.\pipe\my_service";
    
    // HTTP-style client for request/response
    let client = IpcHttpClient::new(ipc_path)?;
    
    // 🔥 New fluent API - like reqwest!
    let response = client
        .get("/api/version")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    
    println!("Status: {}", response.status());
    println!("Success: {}", response.is_success());
    
    Ok(())
}
```

### Basic Server Usage
```rust
use kode_bridge::{IpcHttpServer, Router, HttpResponse, Result};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Create HTTP server with routing
    let router = Router::new()
        .get("/health", |_| async move {
            HttpResponse::json(&json!({"status": "healthy"}))
        })
        .post("/api/data", |ctx| async move {
            let data: serde_json::Value = ctx.json()?;
            HttpResponse::json(&json!({"received": data}))
        });

    let mut server = IpcHttpServer::new("/tmp/server.sock")?
        .router(router);
    
    println!("🚀 Server listening on /tmp/server.sock");
    server.serve().await
}
```