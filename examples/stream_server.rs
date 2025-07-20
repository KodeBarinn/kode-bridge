//! Streaming IPC server example
//!
//! This example demonstrates how to create a streaming IPC server that
//! broadcasts real-time data to multiple connected clients.

use kode_bridge::{IpcStreamServer, JsonDataSource, Result, StreamMessage, StreamServerConfig};
use serde_json::json;
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::signal;
use tracing::info;

#[derive(serde::Serialize)]
struct TrafficData {
    timestamp: u64,
    up: u64,
    down: u64,
    connections: u32,
}

#[derive(serde::Serialize)]
struct SystemMetrics {
    timestamp: u64,
    cpu_usage: f64,
    memory_usage: f64,
    disk_usage: f64,
    network_rx: u64,
    network_tx: u64,
}

#[derive(serde::Serialize)]
struct EventLog {
    timestamp: u64,
    level: String,
    message: String,
    source: String,
}

fn generate_traffic_data() -> Result<serde_json::Value> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let traffic = TrafficData {
        timestamp,
        up: rand::random::<u64>() % 1000000, // Random upload bytes
        down: rand::random::<u64>() % 5000000, // Random download bytes
        connections: rand::random::<u32>() % 100 + 10, // 10-110 connections
    };

    Ok(serde_json::to_value(traffic)?)
}

fn generate_system_metrics() -> Result<serde_json::Value> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let metrics = SystemMetrics {
        timestamp,
        cpu_usage: (rand::random::<f64>() * 100.0).round() / 100.0, // 0-100%
        memory_usage: (rand::random::<f64>() * 100.0).round() / 100.0, // 0-100%
        disk_usage: (rand::random::<f64>() * 100.0).round() / 100.0, // 0-100%
        network_rx: rand::random::<u64>() % 1000000,
        network_tx: rand::random::<u64>() % 1000000,
    };

    Ok(serde_json::to_value(metrics)?)
}

fn generate_event_log() -> Result<serde_json::Value> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let events = vec![
        ("INFO", "User logged in successfully", "auth"),
        ("WARN", "High memory usage detected", "system"),
        ("ERROR", "Failed to connect to database", "database"),
        ("INFO", "Backup completed successfully", "backup"),
        ("DEBUG", "Cache refresh initiated", "cache"),
        ("INFO", "New client connected", "network"),
        ("WARN", "Rate limit exceeded", "api"),
    ];

    let (level, message, source) = events[rand::random::<usize>() % events.len()];

    let event = EventLog {
        timestamp,
        level: level.to_string(),
        message: message.to_string(),
        source: source.to_string(),
    };

    Ok(serde_json::to_value(event)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("🌊 Streaming IPC Server Example");
    println!("===============================");

    // Get IPC path from environment or use default
    #[cfg(unix)]
    let ipc_path =
        env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/stream_server.sock".to_string());
    #[cfg(windows)]
    let ipc_path =
        env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\stream_server".to_string());

    println!("📡 Server will listen on: {}", ipc_path);

    // Configure server
    let config = StreamServerConfig {
        max_connections: 100,
        buffer_size: 65536,
        write_timeout: Duration::from_secs(10),
        max_message_size: 1024 * 1024, // 1MB
        enable_logging: true,
        shutdown_timeout: Duration::from_secs(5),
        broadcast_capacity: 1000,
        keepalive_interval: Duration::from_secs(30),
    };

    // Create data source that generates traffic data every 2 seconds
    let traffic_source = JsonDataSource::new(generate_traffic_data, Duration::from_secs(2));

    // Create server
    let mut server = IpcStreamServer::with_config(&ipc_path, config)?;

    println!("🌟 Server configured for streaming:");
    println!("  📊 Traffic data every 2 seconds");
    println!("  💾 System metrics");
    println!("  📝 Event logs");
    println!("  🔄 Keep-alive pings");
    println!();

    // Start background tasks for additional data streams
    let server_broadcast = {
        // Get a clone of the broadcast functionality
        // Note: In real implementation, you'd get the broadcast sender from the server
        println!("📈 Starting additional data generators...");

        // Simulate system metrics broadcast
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Ok(metrics) = generate_system_metrics() {
                    info!(
                        "Generated system metrics: CPU {:.1}%",
                        metrics
                            .get("cpu_usage")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                    );
                    // In real implementation: server.broadcast(StreamMessage::Json(metrics))?;
                }
            }
        })
    };

    // Event log generator
    let event_generator = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(8));
        loop {
            interval.tick().await;
            if let Ok(event) = generate_event_log() {
                info!(
                    "Generated event: {} - {}",
                    event
                        .get("level")
                        .and_then(|v| v.as_str())
                        .unwrap_or("UNKNOWN"),
                    event
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No message")
                );
                // In real implementation: server.broadcast(StreamMessage::Json(event))?;
            }
        }
    });

    // Start server with traffic data source
    let server_task = tokio::spawn(async move {
        if let Err(e) = server.serve_with_source(traffic_source).await {
            eprintln!("Server error: {}", e);
        }
    });

    println!("✅ Server started successfully!");
    println!("📊 Data streams active:");
    println!("  • Traffic data: Every 2 seconds");
    println!("  • System metrics: Every 5 seconds");
    println!("  • Event logs: Every 8 seconds");
    println!("  • Keep-alive: Every 30 seconds");
    println!();

    println!("📱 Client connection info:");
    #[cfg(unix)]
    {
        println!(
            "CUSTOM_SOCK={} cargo run --features=client --example elegant_stream",
            ipc_path
        );
    }
    #[cfg(windows)]
    {
        println!("set CUSTOM_PIPE={}", ipc_path);
        println!("cargo run --features=client --example elegant_stream");
    }
    println!();

    // Show server stats periodically
    let stats_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            // In real implementation, get stats from server
            info!(
                "📊 Server stats: {} connections, broadcasting data streams",
                0
            );
        }
    });

    // Wait for shutdown signal
    println!("🎯 Server is running. Press Ctrl+C to shutdown...");

    match signal::ctrl_c().await {
        Ok(()) => {
            println!("🛑 Shutdown signal received");
        }
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
        }
    }

    // Shutdown tasks
    println!("🔄 Shutting down server...");
    server_task.abort();
    server_broadcast.abort();
    event_generator.abort();
    stats_task.abort();

    // Give some time for cleanup
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("✅ Server stopped");

    Ok(())
}

// Example of manual broadcasting (for demonstration)
#[allow(dead_code)]
async fn manual_broadcast_example() -> Result<()> {
    // This shows how you might manually broadcast messages
    let _data = json!({
        "type": "notification",
        "message": "Manual broadcast message",
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    });

    // server.broadcast(StreamMessage::Json(data))?;
    info!("Manual broadcast sent");
    Ok(())
}

// Example of different message types
#[allow(dead_code)]
fn demonstrate_message_types() {
    // JSON messages
    let _json_msg = StreamMessage::json(&json!({
        "type": "data",
        "value": 42
    }));

    // Text messages
    let _text_msg = StreamMessage::text("Hello, streaming clients!");

    // Binary messages
    let _binary_msg = StreamMessage::binary(vec![0x01, 0x02, 0x03, 0x04]);

    info!("Demonstrated different message types");
}
