# kode-bridge Cross-Platform Usage Guide

**Complete guide for using the modern HTTP Over IPC library across all platforms**

This comprehensive guide explains how to use kode-bridge's modern fluent API, configuration options, and platform-specific features across Unix/Linux/macOS and Windows systems.

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

## 🌍 Cross-Platform Configuration

### Environment Variables

The library automatically detects your platform and uses appropriate IPC transport:

#### Unix/Linux/macOS (Unix Domain Sockets)
- **Variable**: `CUSTOM_SOCK`
- **Default**: `/tmp/example.sock`
- **Format**: Absolute path to socket file
- **Examples**: 
  ```bash
  # Basic usage
  CUSTOM_SOCK=/tmp/my_service.sock cargo run --example request
  
  # Service-specific paths
  CUSTOM_SOCK=/var/run/my_app/api.sock cargo run --example elegant_http
  CUSTOM_SOCK=/tmp/clash_api.sock cargo run --example traffic
  ```

#### Windows (Named Pipes)
- **Variable**: `CUSTOM_PIPE` 
- **Default**: `\\\\.\\pipe\\example`
- **Format**: `\\\\.\\pipe\\pipe_name`
- **Examples**:
  ```cmd
  REM Basic usage
  set CUSTOM_PIPE=\\\\.\\pipe\\my_service
  cargo run --example request
  
  REM Service-specific pipes
  set CUSTOM_PIPE=\\\\.\\pipe\\clash_api
  cargo run --example traffic
  
  REM PowerShell
  $env:CUSTOM_PIPE="\\\\.\\pipe\\my_service"
  cargo run --example elegant_http
  ```

### Modern Configuration Options

#### HTTP Client Configuration
```rust
use kode_bridge::{IpcHttpClient, ClientConfig};
use std::time::Duration;

let config = ClientConfig {
    default_timeout: Duration::from_secs(30),
    enable_pooling: true,
    pool_max_size: 10,
    pool_min_idle: 2,
    pool_max_idle_time_ms: 30000,
    max_retries: 3,
    retry_delay: Duration::from_millis(200),
};

let client = IpcHttpClient::with_config(&ipc_path, config)?;
```

#### Streaming Client Configuration
```rust
use kode_bridge::{IpcStreamClient, StreamClientConfig};
use std::time::Duration;

let config = StreamClientConfig {
    default_timeout: Duration::from_secs(60),
    max_retries: 5,
    retry_delay: Duration::from_millis(100),
    buffer_size: 32768, // Large buffer for high-throughput
};

let stream_client = IpcStreamClient::with_config(&ipc_path, config)?;
```

## 📄 Environment File Configuration

### .env File Support

Create a `.env` file in your project root for persistent configuration:

#### Unix/Linux/macOS .env example:
```env
# Unix Domain Socket configuration
CUSTOM_SOCK=/tmp/my_service.sock

# Alternative paths
# CUSTOM_SOCK=/var/run/my_app/api.sock
# CUSTOM_SOCK=/tmp/clash_api.sock
```

#### Windows .env example:
```env
# Named Pipe configuration (note: each backslash doubled for escaping)
CUSTOM_PIPE=\\\\\\\\.\\\\pipe\\\\my_service

# Alternative pipes
# CUSTOM_PIPE=\\\\\\\\.\\\\pipe\\\\clash_api
# CUSTOM_PIPE=\\\\\\\\.\\\\pipe\\\\app_communication
```

### Advanced Environment Configuration

#### Development vs Production
```env
# Development
CUSTOM_SOCK=/tmp/dev_service.sock
LOG_LEVEL=debug
ENABLE_POOLING=true
MAX_RETRIES=3

# Production (different .env file)
CUSTOM_SOCK=/var/run/production/api.sock
LOG_LEVEL=info
ENABLE_POOLING=true
MAX_RETRIES=5
POOL_SIZE=20
```

#### Using Environment in Code
```rust
use dotenv::dotenv;
use std::env;
use kode_bridge::{ClientConfig, IpcHttpClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    
    // Platform-specific path resolution
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")
        .unwrap_or_else(|_| "/tmp/default.sock".to_string());
    
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")
        .unwrap_or_else(|_| r"\\\\.\\pipe\\default".to_string());
    
    // Environment-driven configuration
    let enable_pooling = env::var("ENABLE_POOLING")
        .unwrap_or_else(|_| "true".to_string())
        .parse::<bool>()
        .unwrap_or(true);
    
    let config = ClientConfig {
        enable_pooling,
        ..Default::default()
    };
    
    let client = IpcHttpClient::with_config(&ipc_path, config)?;
    
    Ok(())
}
```

## 🔧 Platform-Specific Features and Optimizations

### Unix/Linux/macOS (Unix Domain Sockets)

#### Advantages
- **Superior Performance**: Direct kernel communication without network stack
- **Connection Pooling**: Excellent connection reuse and pooling performance
- **File System Integration**: Socket files integrate with standard file permissions
- **Process Communication**: Ideal for local service communication

#### Configuration
- **Default Path**: `/tmp/example.sock`
- **Permissions**: Automatically handles socket file permissions
- **Cleanup**: Automatic socket file cleanup on client shutdown

#### Best Practices
```bash
# Ensure socket directory exists and has proper permissions
sudo mkdir -p /var/run/my_app
sudo chown $USER:$USER /var/run/my_app

# Use service-specific paths
CUSTOM_SOCK=/var/run/my_app/api.sock

# Development vs production paths
# Development: /tmp/dev_app.sock
# Production: /var/run/production/app.sock
```

### Windows (Named Pipes)

#### Advantages
- **Native Windows IPC**: Optimized for Windows kernel architecture
- **Security Integration**: Integrates with Windows security model
- **Service Communication**: Excellent for Windows service communication
- **Cross-Session**: Can communicate across user sessions when configured

#### Configuration
- **Default Path**: `\\\\.\\pipe\\example`
- **Namespace**: Uses the `\\\\.\\pipe\\` namespace
- **Security**: Automatic security descriptor management

#### Best Practices
```cmd
REM Use descriptive pipe names
set CUSTOM_PIPE=\\\\.\\pipe\\my_app_api

REM Service-specific naming
set CUSTOM_PIPE=\\\\.\\pipe\\clash_monitoring
set CUSTOM_PIPE=\\\\.\\pipe\\system_metrics

REM PowerShell configuration
$env:CUSTOM_PIPE="\\\\.\\pipe\\my_service"
```

### Performance Characteristics

#### Connection Pool Performance
| Platform | Pool Efficiency | Connection Reuse | Throughput |
|----------|----------------|------------------|------------|
| Unix     | Excellent      | 95%+             | High       |
| Windows  | Very Good      | 90%+             | High       |

#### Benchmark Results
```bash
# Unix typically shows:
# - Higher connection reuse rates
# - Lower latency for frequent requests
# - Better streaming performance

# Windows shows:
# - Stable performance across scenarios
# - Good security integration
# - Reliable service communication
```

## 🔄 Migration and Modernization Guide

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
kode-bridge = "0.1"
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

## 📚 Additional Resources

### Example Guides
- **[Examples Guide](./EXAMPLES_GUIDE.md)** - Comprehensive examples with modern patterns
- **[Examples Directory](./examples/)** - Complete runnable examples

### Documentation
- **[API Documentation](https://docs.rs/kode-bridge)** - Complete API reference
- **[Crate Page](https://crates.io/crates/kode-bridge)** - Latest version and stats

### Community
- **[GitHub Issues](https://github.com/KodeBarinn/kode-bridge/issues)** - Bug reports and feature requests
- **[GitHub Discussions](https://github.com/KodeBarinn/kode-bridge/discussions)** - Community discussions

---

**kode-bridge** - Modern HTTP Over IPC for Rust 🚀