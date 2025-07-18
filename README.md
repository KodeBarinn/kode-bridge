# kode-bridge

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](http://www.apache.org/licenses/LICENSE-2.0)
[![Crates.io](https://img.shields.io/crates/v/kode-bridge.svg)](https://crates.io/crates/kode-bridge)

**[中文](./README_CN.md) | English**

**kode-bridge** is a modern Rust library designed for cross-platform (macOS, Linux, Windows) IPC communication. It provides both HTTP-style request/response and real-time streaming capabilities through Unix Domain Sockets or Windows Named Pipes, with a fluent API similar to reqwest.

## ✨ Features

- **🌍 True Cross-Platform**: Automatically detects the platform and uses optimal IPC methods
  - **Unix/Linux/macOS**: Unix Domain Sockets
  - **Windows**: Named Pipes
- **🚀 Dual Client Architecture**: 
  - **`IpcHttpClient`**: HTTP-style request/response for API calls
  - **`IpcStreamClient`**: Real-time streaming for continuous data monitoring
- **💎 Fluent API**: Reqwest-inspired method chaining with type-safe JSON handling
- **📦 Auto Serialization**: Built-in JSON request and response processing
- **⚡ High Performance**: Optimized connection management strategies for different platforms
- **🔧 Easy Integration**: Based on [interprocess](https://github.com/kotauskas/interprocess) and Tokio async runtime
- **🔄 Backward Compatible**: Old API methods still work alongside new fluent interface
- **📖 Complete Support**: Includes examples, benchmarks, and comprehensive documentation

## 🚀 Quick Start

### Add Dependencies

```toml
[dependencies]
kode-bridge = "0.1"
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
```

### Basic Usage

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
    
    // Type-safe JSON parsing
    #[derive(serde::Deserialize)]
    struct ApiResponse {
        version: String,
        meta: bool,
    }
    
    let data: ApiResponse = response.json()?;
    println!("Version: {}", data.version);
    
    // POST with JSON body
    let update_data = json!({"user": "alice", "action": "login"});
    let response = client
        .post("/api/auth")
        .json_body(&update_data)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    
    if response.is_success() {
        println!("Auth successful!");
    }
    
    // Real-time streaming client
    let stream_client = IpcStreamClient::new(ipc_path)?;
    
    // Monitor traffic data in real-time
    #[derive(serde::Deserialize, Debug)]
    struct TrafficData {
        up: u64,
        down: u64,
    }
    
    let traffic_data: Vec<TrafficData> = stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(5))
        .json_results()
        .await?;
    
    println!("Collected {} traffic samples", traffic_data.len());
    
    Ok(())
}
```

### Advanced Usage

```rust
use kode_bridge::{IpcHttpClient, IpcStreamClient};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = IpcHttpClient::new("/tmp/service.sock")?;
    
    // All HTTP methods supported
    let response = client.put("/api/config")
        .json_body(&json!({"key": "value"}))
        .send()
        .await?;
    
    // Rich response inspection
    println!("Status: {}", response.status());
    println!("Headers: {:?}", response.headers());
    println!("Content length: {}", response.content_length());
    println!("Is client error: {}", response.is_client_error());
    println!("Is server error: {}", response.is_server_error());
    
    // Stream processing with real-time callbacks
    let stream_client = IpcStreamClient::new("/tmp/service.sock")?;
    
    stream_client
        .get("/events")
        .send()
        .await?
        .process_lines(|line| {
            println!("Real-time event: {}", line);
            Ok(())
        })
        .await?;
    
    Ok(())
}
```

### Using Environment Variables

Create a `.env` file:

```env
# Unix systems
CUSTOM_SOCK=/tmp/my_app.sock

# Windows systems (each backslash needs to be escaped by doubling)
CUSTOM_PIPE=\\\\.\\pipe\\\my_app
```

Then in your code:

```rust
use dotenv::dotenv;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    
    #[cfg(unix)]
    let path = env::var("CUSTOM_SOCK").unwrap_or("/tmp/default.sock".to_string());
    
    #[cfg(windows)]
    let path = env::var("CUSTOM_PIPE").unwrap_or(r"\\.\pipe\default".to_string());
    
    let client = IpcHttpClient::new(&path)?;
    let response = client.request("GET", "/status", None).await?;
    
    Ok(())
}
```

## 📋 Examples

Run built-in examples:

```bash
# Basic request example
cargo run --example request

# Large data request example  
cargo run --example request_large

# Elegant HTTP client demo
cargo run --example elegant_http

# Elegant streaming client demo
cargo run --example elegant_stream

# Two client comparison
cargo run --example two_clients

# Real-time traffic monitoring
cargo run --example traffic

# Using custom IPC path
CUSTOM_SOCK=/tmp/my.sock cargo run --example request  # Unix
CUSTOM_PIPE=\\\\.\\pipe\\my_pipe cargo run --example request  # Windows
```

## 🔥 Performance Benchmarks

Run performance benchmarks:

```bash
# Run all benchmarks
cargo bench

# View benchmark reports
open target/criterion/report/index.html
```

Benchmarks automatically:
- Detect the running platform
- Use appropriate environment variables (`CUSTOM_SOCK` or `CUSTOM_PIPE`)
- Apply platform-specific performance optimization strategies

## 🏗️ Architecture Design

```
┌─────────────────────────────────────────┐
│     IpcHttpClient    IpcStreamClient    │
│   (Request/Response)  (Real-time Stream)│
├─────────────────────────────────────────┤
│              Fluent API                 │
│   (HTTP-like Methods & Method Chaining) │
├─────────────────────────────────────────┤
│            http_client.rs               │
│        (HTTP Protocol Handler)          │
├─────────────────────────────────────────┤
│             interprocess                │
│       (Cross-Platform IPC Transport)    │
├─────────────────┬───────────────────────┤
│   Unix Sockets  │    Windows Pipes      │
│   (Unix/Linux)  │     (Windows)         │
└─────────────────┴───────────────────────┘
```

### Core Components

- **`IpcHttpClient`**: HTTP-style request/response client with fluent API
- **`IpcStreamClient`**: Real-time streaming client for continuous data monitoring
- **Fluent API**: Method chaining with `get()`, `post()`, `timeout()`, `json_body()`, `send()`, etc.
- **`http_client`**: Platform-agnostic HTTP protocol handling with chunked transfer encoding support
- **Smart Platform Detection**: Compile-time automatic selection of optimal IPC implementation

### API Comparison

| Feature | Old API | New Fluent API |
|---------|---------|---------------|
| GET Request | `client.request("GET", "/path", None)` | `client.get("/path").send()` |
| POST with JSON | `client.request("POST", "/path", Some(&json))` | `client.post("/path").json_body(&json).send()` |
| Timeout | Not supported | `client.get("/path").timeout(Duration::from_secs(5)).send()` |
| Response Status | `response.status` | `response.status()`, `response.is_success()` |
| JSON Parsing | `response.json()?` | `response.json::<T>()?` with type inference |
| Streaming | Not available | `stream_client.get("/events").json_results().await?` |

## 🎯 Use Cases

- **Local Service Communication**: Communicate with local processes like Clash, Mihomo, proxy services, etc.
- **Real-time Monitoring**: Stream traffic data, logs, metrics, and system events in real-time
- **Microservice Architecture**: High-performance inter-process HTTP communication
- **System Integration**: Replace traditional REST API local calls with IPC
- **Performance-Critical Applications**: Scenarios requiring low-latency local communication
- **Configuration Management**: Dynamic configuration updates with immediate feedback

## 🛠️ Development

### Build Project

```bash
git clone https://github.com/KodeBarinn/kode-bridge.git
cd kode-bridge
cargo build
```

### Run Tests

```bash
cargo test
```

### Generate Documentation

```bash
cargo doc --open
```

## 📚 Resources

- [Platform Guide](./PLATFORM_GUIDE.md) - Detailed cross-platform usage guide
- [Examples](./examples/) - Complete example code
- [Benchmarks](./benches/) - Performance benchmarks

## 🤝 Contributing

We welcome Issues and Pull Requests!

## 📄 License

This project is licensed under the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0).

See the [Licence](./Licence) file for details.