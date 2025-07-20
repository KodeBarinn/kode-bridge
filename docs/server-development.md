# Server Development Guide

Complete guide for developing servers with kode-bridge.

## 🚀 Server Features

### HTTP Server (`IpcHttpServer`)
- **Express.js-style routing**: Familiar patterns for web developers
- **Middleware support**: Request/response interceptors
- **JSON handling**: Automatic serialization/deserialization
- **Error handling**: Structured error responses
- **Static file serving**: Built-in static content support
- **Request context**: Rich request information access

### Streaming Server (`IpcStreamServer`)
- **Real-time broadcasting**: Push data to multiple clients
- **Client lifecycle management**: Connection tracking and cleanup
- **Multiple data formats**: JSON, text, binary support
- **Backpressure handling**: Automatic client lag detection
- **Custom data sources**: Pluggable data generation

## 📋 HTTP Server Examples

### Basic HTTP Server
```rust
use kode_bridge::{IpcHttpServer, Router, HttpResponse, RequestContext, Result};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let router = Router::new()
        .get("/", |_| async move {
            HttpResponse::ok("Welcome to kode-bridge server!")
        })
        .get("/health", |_| async move {
            HttpResponse::json(&json!({
                "status": "healthy",
                "timestamp": chrono::Utc::now().to_rfc3339()
            }))
        })
        .post("/api/echo", |ctx| async move {
            let body: serde_json::Value = ctx.json()?;
            HttpResponse::json(&json!({
                "echo": body,
                "method": ctx.method(),
                "path": ctx.path()
            }))
        });

    let mut server = IpcHttpServer::new("/tmp/example.sock")?
        .router(router);
    
    println!("🚀 Server listening on /tmp/example.sock");
    server.serve().await
}
```

### Advanced HTTP Server with Middleware
```rust
use kode_bridge::{IpcHttpServer, Router, HttpResponse, RequestContext, Result};
use serde_json::json;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let router = Router::new()
        // Middleware: Request logging
        .middleware(|ctx, next| async move {
            let start = Instant::now();
            println!("→ {} {}", ctx.method(), ctx.path());
            
            let response = next(ctx).await;
            
            let duration = start.elapsed();
            println!("← {} {} ({:?})", 
                response.status(), ctx.path(), duration);
            
            response
        })
        // Routes
        .get("/api/users/:id", |ctx| async move {
            let user_id = ctx.param("id").unwrap_or("unknown");
            HttpResponse::json(&json!({
                "user_id": user_id,
                "name": format!("User {}", user_id)
            }))
        })
        .post("/api/users", |ctx| async move {
            #[derive(serde::Deserialize)]
            struct CreateUser {
                name: String,
                email: String,
            }
            
            let user: CreateUser = ctx.json()?;
            
            // Simulate user creation
            let new_user = json!({
                "id": 123,
                "name": user.name,
                "email": user.email,
                "created_at": chrono::Utc::now().to_rfc3339()
            });
            
            HttpResponse::json(&new_user).status(201)
        })
        .put("/api/config", |ctx| async move {
            let config: serde_json::Value = ctx.json()?;
            
            // Validate configuration
            if config.get("version").is_none() {
                return HttpResponse::bad_request("Missing version field");
            }
            
            HttpResponse::json(&json!({
                "message": "Configuration updated",
                "config": config
            }))
        })
        .delete("/api/cache", |_| async move {
            // Simulate cache clearing
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            HttpResponse::json(&json!({
                "message": "Cache cleared successfully"
            }))
        });

    let mut server = IpcHttpServer::new("/tmp/api_server.sock")?
        .router(router)
        .max_connections(100)
        .request_timeout(std::time::Duration::from_secs(30));
    
    println!("🚀 API Server listening on /tmp/api_server.sock");
    server.serve().await
}
```

## 📡 Streaming Server Examples

### Basic Streaming Server
```rust
use kode_bridge::{IpcStreamServer, StreamSource, Result};
use serde_json::json;
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt;

struct MetricsSource;

#[async_trait::async_trait]
impl StreamSource for MetricsSource {
    async fn generate_stream(&self) -> Result<Box<dyn Stream<Item = String> + Send + Unpin>> {
        let stream = IntervalStream::new(interval(Duration::from_secs(1)))
            .map(|_| {
                let metrics = json!({
                    "cpu_usage": rand::random::<f64>() * 100.0,
                    "memory_usage": rand::random::<f64>() * 100.0,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });
                metrics.to_string()
            });
        
        Ok(Box::new(stream))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut server = IpcStreamServer::new("/tmp/metrics.sock")?
        .add_source("/metrics", Box::new(MetricsSource))
        .client_timeout(Duration::from_secs(60));
    
    println!("📡 Streaming server listening on /tmp/metrics.sock");
    server.serve().await
}
```

### Advanced Streaming Server with Multiple Sources
```rust
use kode_bridge::{IpcStreamServer, StreamSource, Result};
use serde_json::json;
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;

// Traffic monitoring source
struct TrafficSource {
    counters: Arc<RwLock<(u64, u64)>>, // (upload, download)
}

#[async_trait::async_trait]
impl StreamSource for TrafficSource {
    async fn generate_stream(&self) -> Result<Box<dyn Stream<Item = String> + Send + Unpin>> {
        let counters = self.counters.clone();
        
        let stream = IntervalStream::new(interval(Duration::from_millis(500)))
            .map(move |_| {
                let counters = counters.clone();
                async move {
                    let mut guard = counters.write().await;
                    guard.0 += rand::random::<u64>() % 1000000; // Random upload
                    guard.1 += rand::random::<u64>() % 2000000; // Random download
                    
                    let traffic = json!({
                        "upload": guard.0,
                        "download": guard.1,
                        "upload_rate": rand::random::<u64>() % 10000,
                        "download_rate": rand::random::<u64>() % 20000,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    });
                    
                    traffic.to_string()
                }
            })
            .then(|future| future);
        
        Ok(Box::new(stream))
    }
}

// System events source
struct EventsSource;

#[async_trait::async_trait]
impl StreamSource for EventsSource {
    async fn generate_stream(&self) -> Result<Box<dyn Stream<Item = String> + Send + Unpin>> {
        let events = vec![
            "connection_established",
            "data_processed", 
            "cache_miss",
            "cache_hit",
            "error_recovered",
            "backup_completed",
        ];
        
        let stream = IntervalStream::new(interval(Duration::from_secs(2)))
            .map(move |_| {
                let event_type = &events[rand::random::<usize>() % events.len()];
                let event = json!({
                    "type": event_type,
                    "severity": if event_type.contains("error") { "high" } else { "normal" },
                    "message": format!("Event: {}", event_type),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });
                event.to_string()
            });
        
        Ok(Box::new(stream))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let traffic_counters = Arc::new(RwLock::new((0u64, 0u64)));
    
    let mut server = IpcStreamServer::new("/tmp/monitoring.sock")?
        .add_source("/traffic", Box::new(TrafficSource {
            counters: traffic_counters.clone()
        }))
        .add_source("/events", Box::new(EventsSource))
        .add_source("/metrics", Box::new(MetricsSource))
        .client_timeout(Duration::from_secs(120))
        .max_clients(50);
    
    println!("📡 Monitoring server listening on /tmp/monitoring.sock");
    println!("Available streams:");
    println!("  /traffic - Real-time traffic data");
    println!("  /events  - System events");
    println!("  /metrics - Performance metrics");
    
    server.serve().await
}
```

## 🛠️ Server Configuration

### HTTP Server Configuration
```rust
use kode_bridge::{IpcHttpServer, ServerConfig};
use std::time::Duration;

let config = ServerConfig {
    max_connections: 200,
    request_timeout: Duration::from_secs(30),
    keep_alive_timeout: Duration::from_secs(60),
    max_request_size: 1024 * 1024, // 1MB
    enable_compression: true,
};

let mut server = IpcHttpServer::with_config("/tmp/server.sock", config)?
    .router(router);
```

### Streaming Server Configuration  
```rust
use kode_bridge::{IpcStreamServer, StreamServerConfig};
use std::time::Duration;

let config = StreamServerConfig {
    max_clients: 100,
    client_timeout: Duration::from_secs(300),
    buffer_size: 65536,
    heartbeat_interval: Duration::from_secs(30),
    lag_threshold: Duration::from_secs(10),
};

let mut server = IpcStreamServer::with_config("/tmp/stream.sock", config)?;
```

## 🔒 Error Handling and Security

### Structured Error Responses
```rust
use kode_bridge::{HttpResponse, KodeBridgeError};

// Custom error handling
.post("/api/validate", |ctx| async move {
    let data: serde_json::Value = match ctx.json() {
        Ok(data) => data,
        Err(e) => {
            return HttpResponse::bad_request(&format!("Invalid JSON: {}", e));
        }
    };
    
    // Validate required fields
    if data.get("required_field").is_none() {
        return HttpResponse::unprocessable_entity("Missing required_field");
    }
    
    HttpResponse::ok("Validation passed")
})
```

### Input Validation and Sanitization
```rust
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct UserInput {
    #[serde(deserialize_with = "validate_username")]
    username: String,
    
    #[serde(deserialize_with = "validate_email")]
    email: String,
}

fn validate_username<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let username = String::deserialize(deserializer)?;
    
    if username.len() < 3 || username.len() > 20 {
        return Err(serde::de::Error::custom("Username must be 3-20 characters"));
    }
    
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(serde::de::Error::custom("Username contains invalid characters"));
    }
    
    Ok(username)
}
```

## 📈 Performance Optimization

### Connection Management
```rust
// Optimize for high-concurrency scenarios
let mut server = IpcHttpServer::new("/tmp/server.sock")?
    .max_connections(500)  // Increase concurrent connections
    .request_timeout(Duration::from_secs(15))  // Shorter timeout
    .keep_alive_timeout(Duration::from_secs(30));  // Connection reuse
```

### Memory Management
```rust
// For streaming servers with high throughput
let mut server = IpcStreamServer::new("/tmp/stream.sock")?
    .buffer_size(128 * 1024)  // 128KB buffer for high-volume data
    .max_clients(200)         // Limit clients to prevent memory exhaustion
    .lag_threshold(Duration::from_secs(5));  // Disconnect slow clients
```

### Monitoring and Metrics
```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
struct ServerMetrics {
    requests_total: Arc<AtomicU64>,
    requests_success: Arc<AtomicU64>,
    requests_error: Arc<AtomicU64>,
}

impl ServerMetrics {
    fn new() -> Self {
        Self {
            requests_total: Arc::new(AtomicU64::new(0)),
            requests_success: Arc::new(AtomicU64::new(0)),
            requests_error: Arc::new(AtomicU64::new(0)),
        }
    }
    
    fn record_request(&self, success: bool) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.requests_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.requests_error.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    fn get_stats(&self) -> (u64, u64, u64) {
        (
            self.requests_total.load(Ordering::Relaxed),
            self.requests_success.load(Ordering::Relaxed),
            self.requests_error.load(Ordering::Relaxed),
        )
    }
}
```