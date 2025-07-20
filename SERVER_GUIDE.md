# kode-bridge Server Guide

**Complete guide for building HTTP Over IPC servers**

This guide explains how to use kode-bridge's server functionality to create high-performance IPC servers that handle HTTP-style requests and real-time streaming.

## 🚀 Quick Start

### Enable Server Features

Add to your `Cargo.toml`:

```toml
[dependencies]
kode-bridge = { version = "0.1", features = ["server"] }
# Or for both client and server
kode-bridge = { version = "0.1", features = ["full"] }

# Required dependencies for examples
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing-subscriber = "0.3"
```

### Available Features

- `client` (default) - Enables IPC client functionality
- `server` - Enables IPC server functionality  
- `full` - Enables both client and server

## 🌐 HTTP IPC Server

### Basic HTTP Server

```rust
use kode_bridge::{IpcHttpServer, Router, HttpResponse, ServerConfig, Result};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure server
    let config = ServerConfig {
        max_connections: 100,
        read_timeout: Duration::from_secs(30),
        write_timeout: Duration::from_secs(30),
        max_request_size: 1024 * 1024, // 1MB
        enable_logging: true,
        shutdown_timeout: Duration::from_secs(5),
    };

    // Create router with endpoints
    let router = Router::new()
        .get("/health", |_ctx| async move {
            HttpResponse::json(&json!({"status": "healthy"}))
        })
        .post("/api/data", |ctx| async move {
            match ctx.json::<serde_json::Value>() {
                Ok(data) => {
                    println!("Received: {}", data);
                    HttpResponse::json(&json!({"received": data}))
                }
                Err(e) => Ok(HttpResponse::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .json(&json!({"error": e.to_string()}))?
                    .build())
            }
        });

    // Start server
    let mut server = IpcHttpServer::with_config("/tmp/my_server.sock", config)?
        .router(router);
    
    server.serve().await
}
```

### HTTP Server Features

#### Request Context
```rust
use kode_bridge::RequestContext;

// Handler function receives RequestContext
let handler = |ctx: RequestContext| async move {
    // Access request details
    println!("Method: {}", ctx.method);
    println!("URI: {}", ctx.uri);
    println!("Headers: {:?}", ctx.headers);
    
    // Parse JSON body
    if let Ok(data) = ctx.json::<MyStruct>() {
        // Process data
    }
    
    // Get query parameters
    let params = ctx.query_params();
    
    // Get body as text
    let text = ctx.text()?;
    
    HttpResponse::ok()
};
```

#### Response Building
```rust
use kode_bridge::{HttpResponse, ResponseBuilder};
use http::StatusCode;

// Simple responses
let response = HttpResponse::ok();
let response = HttpResponse::not_found();
let response = HttpResponse::internal_error();

// JSON responses
let response = HttpResponse::json(&json!({"key": "value"}))?;

// Custom responses
let response = HttpResponse::builder()
    .status(StatusCode::CREATED)
    .header("content-type", "application/json")
    .json(&data)?
    .build();

// Text responses
let response = HttpResponse::text("Hello, World!");

// Error responses
let response = HttpResponse::error(StatusCode::BAD_REQUEST, "Invalid input");
```

#### Routing
```rust
use kode_bridge::Router;

let router = Router::new()
    // HTTP methods
    .get("/users", get_users)
    .post("/users", create_user)
    .put("/users/{id}", update_user)  // Path parameters coming soon
    .delete("/users/{id}", delete_user)
    
    // Async closures
    .get("/status", |_| async { HttpResponse::text("OK") })
    
    // Complex handlers
    .post("/upload", |ctx| async move {
        let body_size = ctx.body.len();
        if body_size > 1024 * 1024 {
            return Ok(HttpResponse::error(
                StatusCode::PAYLOAD_TOO_LARGE, 
                "File too large"
            ));
        }
        
        // Process upload
        HttpResponse::json(&json!({
            "uploaded": true,
            "size": body_size
        }))
    });
```

#### Server Configuration
```rust
use kode_bridge::ServerConfig;
use std::time::Duration;

let config = ServerConfig {
    // Connection limits
    max_connections: 200,
    
    // Timeouts
    read_timeout: Duration::from_secs(30),
    write_timeout: Duration::from_secs(30),
    shutdown_timeout: Duration::from_secs(10),
    
    // Limits
    max_request_size: 5 * 1024 * 1024, // 5MB
    
    // Logging
    enable_logging: true,
};
```

#### Server Statistics
```rust
// Get server statistics
let stats = server.stats();
println!("Connections: {}", stats.total_connections);
println!("Active: {}", stats.active_connections);  
println!("Requests: {}", stats.total_requests);
println!("Uptime: {:?}", stats.started_at.elapsed());
```

## 🌊 Streaming IPC Server

### Basic Streaming Server

```rust
use kode_bridge::{
    IpcStreamServer, StreamMessage, StreamServerConfig, 
    JsonDataSource, Result
};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure streaming server
    let config = StreamServerConfig {
        max_connections: 500,
        buffer_size: 65536,
        write_timeout: Duration::from_secs(10),
        broadcast_capacity: 1000,
        keepalive_interval: Duration::from_secs(30),
        ..Default::default()
    };

    // Create data source that generates data every 2 seconds
    let data_source = JsonDataSource::new(
        || {
            Ok(json!({
                "timestamp": chrono::Utc::now().timestamp(),
                "data": rand::random::<f64>(),
                "status": "active"
            }))
        },
        Duration::from_secs(2)
    );

    // Create and start server
    let mut server = IpcStreamServer::with_config("/tmp/stream_server.sock", config)?;
    server.serve_with_source(data_source).await
}
```

### Streaming Features

#### Message Types
```rust
use kode_bridge::StreamMessage;

// JSON messages
let msg = StreamMessage::json(&json!({"type": "data", "value": 42}))?;

// Text messages  
let msg = StreamMessage::text("Hello, streaming clients!");

// Binary messages
let msg = StreamMessage::binary(vec![0x01, 0x02, 0x03]);

// Keep-alive ping
let msg = StreamMessage::Ping;

// Server shutdown notification
let msg = StreamMessage::Close;
```

#### Manual Broadcasting
```rust
// Start server without automatic data source
let mut server = IpcStreamServer::new("/tmp/stream.sock")?;

// Start server in background
tokio::spawn(async move {
    server.serve().await
});

// Broadcast messages manually
server.broadcast(StreamMessage::text("Manual message"))?;
server.broadcast(StreamMessage::json(&json!({"event": "update"}))?)?;
```

#### Custom Data Sources
```rust
use kode_bridge::{StreamSource, StreamMessage, Result};
use std::pin::Pin;
use std::future::Future;

struct CustomDataSource {
    counter: u64,
}

impl StreamSource for CustomDataSource {
    fn next_messages(&mut self) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>> {
        Box::pin(async move {
            self.counter += 1;
            let message = StreamMessage::json(&json!({
                "counter": self.counter,
                "timestamp": chrono::Utc::now().timestamp()
            }))?;
            Ok(vec![message])
        })
    }

    fn has_more(&self) -> bool {
        true // Always generate more data
    }

    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            println!("Data source initialized");
            Ok(())
        })
    }

    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            println!("Data source cleaned up");
            Ok(())
        })
    }
}
```

#### Client Management
```rust
// Get connected clients
let clients = server.clients();
for client in clients {
    println!("Client {}: {} messages sent", 
             client.client_id, client.messages_sent);
}

// Get server statistics
let stats = server.stats();
println!("Streaming stats: {}", stats);
println!("Messages/sec: {:.1}", stats.messages_per_second);
```

## 🔧 Cross-Platform Usage

### Unix/Linux/macOS
```bash
# Start HTTP server
cargo run --features=server --example http_server

# Test with client
CUSTOM_SOCK=/tmp/my_server.sock cargo run --features=client --example request
```

### Windows
```cmd
REM Start HTTP server  
cargo run --features=server --example http_server

REM Test with client
set CUSTOM_PIPE=\\.\pipe\my_server
cargo run --features=client --example request
```

## 🎯 Production Deployment

### Error Handling
```rust
use kode_bridge::{HttpResponse, KodeBridgeError};
use http::StatusCode;

let handler = |ctx| async move {
    match process_request(ctx).await {
        Ok(result) => HttpResponse::json(&result),
        Err(e) => {
            tracing::error!("Request failed: {}", e);
            
            let status = if e.to_string().contains("validation") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            
            Ok(HttpResponse::error(status, "Request failed"))
        }
    }
};
```

### Graceful Shutdown
```rust
use tokio::signal;

// Start server in background
let mut server = IpcHttpServer::new("/tmp/server.sock")?;
let server_task = tokio::spawn(async move {
    server.serve().await
});

// Wait for shutdown signal
signal::ctrl_c().await?;
println!("Shutting down server...");

// Stop server
server_task.abort();

// Wait for cleanup
tokio::time::sleep(Duration::from_millis(500)).await;
println!("Server stopped");
```

### Monitoring and Metrics
```rust
// Periodic stats reporting
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let stats = server.stats();
        tracing::info!("Server stats: {}", stats);
        
        // Send to monitoring system
        send_metrics(&stats).await;
    }
});
```

## 📊 Performance Tips

### HTTP Server Optimization
- Use connection pooling on the client side
- Configure appropriate timeouts
- Limit request sizes to prevent memory issues
- Enable request logging only in development

### Streaming Server Optimization  
- Adjust buffer sizes based on message frequency
- Use appropriate broadcast channel capacity
- Implement backpressure for slow clients
- Monitor client lag and disconnect slow clients

### General Tips
- Use `tracing` for structured logging
- Monitor connection counts and memory usage
- Implement health check endpoints
- Use appropriate IPC paths (`/var/run` for production on Unix)

## 🔍 Debugging

### Enable Detailed Logging
```rust
// Add to Cargo.toml dev-dependencies
tracing-subscriber = "0.3"

// In your main function
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();
```

### Common Issues
- **Permission denied**: Check socket file permissions
- **Address already in use**: Ensure socket file is cleaned up
- **Connection refused**: Verify server is running and path is correct
- **Message too large**: Check `max_request_size` and `max_message_size`

## 📚 Examples

See the `examples/` directory for complete working examples:

- `examples/http_server.rs` - Full HTTP server with routing
- `examples/stream_server.rs` - Real-time streaming server
- Run with: `cargo run --features=server --example http_server`

---

**kode-bridge** - Modern HTTP Over IPC for Rust 🚀