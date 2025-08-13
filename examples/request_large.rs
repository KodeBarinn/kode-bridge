/// This example demonstrates how to use `IpcHttpClient` from the `kode_bridge` crate to send an HTTP GET request
/// over an IPC transport (Unix socket on Unix-like systems, Named Pipe on Windows).
///
/// The IPC path is determined by platform-specific environment variables:
/// - Unix: `CUSTOM_SOCK` (e.g., "/tmp/custom.sock")  
/// - Windows: `CUSTOM_PIPE` (e.g., "\\.\pipe\my_pipe")
///
/// The client sends a GET request to the `/proxies` endpoint which typically
/// returns a larger response compared to the `/version` endpoint.
///
/// # Errors
/// Returns an error if the environment variable is missing, the client fails to connect, or the request fails.
///
/// # Example
/// ```env
/// # Unix
/// CUSTOM_SOCK=/tmp/custom.sock
///
/// # Windows  
/// CUSTOM_PIPE=\\.\pipe\my_pipe
/// ```
use dotenvy::dotenv;
use kode_bridge::{ClientConfig, IpcHttpClient, Result};
use serde_json::Value;
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    // Use platform-appropriate environment variable with fallback
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    println!("📡 Large Data Request Example");
    println!("Connecting to: {}", ipc_path);

    // Configure client for large responses
    let config = ClientConfig {
        default_timeout: Duration::from_secs(30), // Longer timeout for large data
        enable_pooling: true,
        pool_config: kode_bridge::pool::PoolConfig {
            max_size: 5,
            min_idle: 2,
            max_idle_time_ms: 300_000,
            ..Default::default()
        },
        max_retries: 2,
        retry_delay: Duration::from_millis(500),
        ..Default::default()
    };

    let client = IpcHttpClient::with_config(&ipc_path, config)?;

    println!("🔄 Sending request to /proxies endpoint...");

    // Use the new fluent API with extended timeout for large responses
    let response = client
        .get("/proxies")
        .timeout(Duration::from_secs(45)) // Extended timeout for large data
        .send()
        .await?;

    println!("✅ Response received!");
    println!("🔍 Response Details:");
    println!("  Status: {}", response.status());
    println!("  Success: {}", response.is_success());
    println!("  Content Length: {} bytes", response.content_length());

    // Handle large JSON responses efficiently
    match response.json_value() {
        Ok(json) => {
            if let Value::Array(ref items) = json {
                println!("📊 Large Data Stats:");
                println!("  Array length: {} items", items.len());
                println!(
                    "  First item preview: {:#}",
                    items.first().unwrap_or(&Value::Null)
                );
                if items.len() > 1 {
                    println!(
                        "  Last item preview: {:#}",
                        items.last().unwrap_or(&Value::Null)
                    );
                }
            } else if let Value::Object(ref obj) = json {
                println!("📊 Large Object Stats:");
                println!("  Object keys: {} properties", obj.len());
                for (key, _) in obj.iter().take(5) {
                    println!("    - {}", key);
                }
                if obj.len() > 5 {
                    println!("    ... and {} more", obj.len() - 5);
                }
            } else {
                println!("📄 JSON Response: {:#}", json);
            }
        }
        Err(e) => {
            let raw_body = response.body()?;
            println!("📄 Raw Response Length: {} bytes", raw_body.len());
            if raw_body.len() > 500 {
                println!("📄 Response Preview: {}...", &raw_body[..500]);
            } else {
                println!("📄 Raw Response: {}", raw_body);
            }
            println!("⚠️  JSON parse error: {}", e);
        }
    }

    // Show pool stats
    if let Some(stats) = client.pool_stats() {
        println!("📊 Connection Pool Stats: {}", stats);
    }

    Ok(())
}
