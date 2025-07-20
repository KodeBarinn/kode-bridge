# Platform Configuration Guide

Complete guide for configuring kode-bridge across different platforms.

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
- **Default**: `\\.\\pipe\\example`
- **Format**: `\\.\\pipe\\pipe_name`
- **Examples**:
  ```cmd
  REM Basic usage
  set CUSTOM_PIPE=\\.\\pipe\\my_service
  cargo run --example request
  
  REM Service-specific pipes
  set CUSTOM_PIPE=\\.\\pipe\\clash_api
  cargo run --example traffic
  
  REM PowerShell
  $env:CUSTOM_PIPE="\\.\\pipe\\my_service"
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
CUSTOM_PIPE=\\\\.\\pipe\\my_service

# Alternative pipes
# CUSTOM_PIPE=\\\\.\\pipe\\clash_api
# CUSTOM_PIPE=\\\\.\\pipe\\app_communication
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
        .unwrap_or_else(|_| r"\\.\\pipe\\default".to_string());
    
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
- **Default Path**: `\\.\\pipe\\example`
- **Namespace**: Uses the `\\.\\pipe\\` namespace
- **Security**: Automatic security descriptor management

#### Best Practices
```cmd
REM Use descriptive pipe names
set CUSTOM_PIPE=\\.\\pipe\\my_app_api

REM Service-specific naming
set CUSTOM_PIPE=\\.\\pipe\\clash_monitoring
set CUSTOM_PIPE=\\.\\pipe\\system_metrics

REM PowerShell configuration
$env:CUSTOM_PIPE="\\.\\pipe\\my_service"
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