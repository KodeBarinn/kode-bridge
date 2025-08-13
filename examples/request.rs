/// This example demonstrates how to use `IpcHttpClient` from the `kode_bridge` crate to send an HTTP GET request
/// over an IPC transport (Unix socket on Unix-like systems, Named Pipe on Windows).
///
/// The IPC path is determined by platform-specific environment variables:
/// - Unix: `CUSTOM_SOCK` (e.g., "/tmp/custom.sock")  
/// - Windows: `CUSTOM_PIPE` (e.g., "\\.\pipe\my_pipe")
///
/// The client sends a GET request to the `/version` endpoint and prints the raw response
/// as well as its JSON representation.
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
use kode_bridge::{IpcHttpClient, Result};
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

    println!("📡 Connecting to: {}", ipc_path);

    // Create client with modern API
    let client = IpcHttpClient::new(&ipc_path)?;

    // Use the new fluent API with timeout
    let response = client
        .get("/version")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    println!("🔍 Response Details:");
    println!("  Status: {}", response.status());
    println!("  Success: {}", response.is_success());
    println!("  Content Length: {}", response.content_length());

    // Parse and display JSON response
    match response.json_value() {
        Ok(json) => println!("📄 JSON Response: {:#}", json),
        Err(e) => {
            println!("📄 Raw Response: {:?}", response.body()?);
            println!("⚠️  JSON parse error: {}", e);
        }
    }

    // Show pool stats if available
    if let Some(stats) = client.pool_stats() {
        println!("📊 Pool Stats: {}", stats);
    }

    Ok(())
}
