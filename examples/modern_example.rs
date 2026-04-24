use kode_bridge::{ClientConfig, IpcHttpClient, IpcStreamClient, Result, StreamClientConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 Modern kode-bridge Example");
    println!("=============================");

    // Example IPC path (would normally be provided by your service)
    #[cfg(unix)]
    let ipc_path = "/tmp/example.sock";
    #[cfg(windows)]
    let ipc_path = r"\\.\pipe\example";

    // Create HTTP client with custom configuration
    let client_config = ClientConfig {
        default_timeout: Duration::from_secs(10),
        enable_pooling: true,
        max_retries: 3,
        retry_delay: Duration::from_millis(200),
        ..Default::default()
    };

    let http_client = IpcHttpClient::with_config(ipc_path, client_config)?;

    // Modern fluent API examples
    println!("\n🔥 HTTP Client Examples:");

    // Note: These examples would require an actual service running
    // They demonstrate the API structure and usage patterns

    println!("  • GET request example:");
    println!("    let response = client.get(\"/api/status\").timeout(Duration::from_secs(5)).send().await?;");

    println!("  • POST with JSON example:");
    println!("    let data = json!({{\"user\": \"alice\", \"action\": \"login\"}});");
    println!("    let response = client.post(\"/api/auth\").json_body(&data).send().await?;");

    println!("  • Response handling example:");
    println!("    if response.is_success() {{");
    println!("        let result: MyData = response.json()?;");
    println!("        println!(\"Success: {{:?}}\", result);");
    println!("    }}");

    // Create streaming client
    let stream_config = StreamClientConfig {
        default_timeout: Duration::from_secs(30),
        max_retries: 3,
        buffer_size: 16384,
        ..Default::default()
    };

    let _stream_client = IpcStreamClient::with_config(ipc_path, stream_config)?;

    println!("\n📡 Streaming Client Examples:");
    println!("  • Stream JSON data example:");
    println!("    let data: Vec<TrafficData> = stream_client");
    println!("        .get(\"/traffic\")");
    println!("        .timeout(Duration::from_secs(10))");
    println!("        .json_results()");
    println!("        .await?;");

    println!("  • Real-time processing example:");
    println!("    stream_client");
    println!("        .get(\"/events\")");
    println!("        .process_lines(|line| {{");
    println!("            println!(\"Event: {{}}\", line);");
    println!("            Ok(())");
    println!("        }})");
    println!("        .await?;");

    // Show pool stats if available
    if let Some(stats) = http_client.pool_stats() {
        println!("\n📊 Connection Pool Stats: {}", stats);
    }
    drop(http_client);

    println!("\n✨ Key Features:");
    println!("  • Modern async/await API with fluent interface");
    println!("  • Connection pooling for better performance");
    println!("  • Comprehensive error handling with KodeBridgeError");
    println!("  • Cross-platform support (Unix sockets + Windows pipes)");
    println!("  • Streaming support for real-time data");
    println!("  • Configurable timeouts and retry logic");
    println!("  • Type-safe JSON serialization/deserialization");
    println!("  • Backward compatibility with legacy APIs");

    println!("\n🎯 Example completed successfully!");

    Ok(())
}
