use dotenvy::dotenv;
use kode_bridge::{ClientConfig, IpcHttpClient, IpcStreamClient, Result, StreamClientConfig};
use std::env;
use std::time::Duration;

// Traffic data structure
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

// Extension trait for convenience
pub trait TrafficMonitorExt {
    fn monitor_traffic(&self, timeout: Duration) -> impl std::future::Future<Output = Result<Vec<TrafficData>>> + Send;
}

impl TrafficMonitorExt for IpcStreamClient {
    async fn monitor_traffic(&self, timeout: Duration) -> Result<Vec<TrafficData>> {
        self.get("/traffic").timeout(timeout).json_results().await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🏗️  Clean Architecture: Two Client Types");
    println!("========================================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    println!("🔧 Testing IpcHttpClient (Request/Response)");

    // Configure HTTP client for API calls
    let http_config = ClientConfig {
        default_timeout: Duration::from_secs(15),
        enable_pooling: true,
        max_retries: 3,
        retry_delay: Duration::from_millis(200),
        ..Default::default()
    };

    let http_client = IpcHttpClient::with_config(&ipc_path, http_config)?;

    // Get basic information - using new elegant API
    let response = http_client
        .get("/proxies")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ Status: {}", response.status());
    println!("📄 Response length: {} bytes", response.content_length());
    println!("✨ Is success: {}", response.is_success());
    println!("🔍 Is client error: {}", response.is_client_error());
    println!("💥 Is server error: {}", response.is_server_error());

    // Parse JSON response
    if response.is_success() {
        match response.json_value() {
            Ok(json_data) => {
                if let Some(proxies_obj) = json_data.as_object() {
                    println!("🔍 Found {} proxy groups", proxies_obj.len());

                    // Show first few proxy names
                    for (i, (name, _)) in proxies_obj.iter().take(3).enumerate() {
                        println!("  {}. {}", i + 1, name);
                    }
                }
            }
            Err(e) => {
                println!("⚠️ JSON parsing failed: {}", e);
                println!("📄 Raw response: {:?}", response.body()?);
            }
        }
    }

    // Test POST request
    println!("\n📤 Testing POST request with JSON body");
    let update_data = serde_json::json!({
        "allow-lan": true,
        "bind-address": "*",
        "port": 7890
    });

    let post_response = http_client
        .post("/configs")
        .json_body(&update_data)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ POST Status: {}", post_response.status());

    println!("\n🌊 Testing IpcStreamClient (Streaming)");

    // Configure streaming client for real-time data
    let stream_config = StreamClientConfig {
        default_timeout: Duration::from_secs(30),
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
        buffer_size: 16384,
    };

    let stream_client = IpcStreamClient::with_config(&ipc_path, stream_config)?;

    // Stream monitoring - using new elegant API
    let traffic_data: Vec<TrafficData> = stream_client
        .monitor_traffic(Duration::from_secs(6))
        .await?;

    println!("✅ Collected {} traffic samples", traffic_data.len());

    if let Some(latest) = traffic_data.last() {
        println!(
            "📊 Latest: ⬆️ {} ⬇️ {}",
            format_bytes(latest.up),
            format_bytes(latest.down)
        );
    }

    if traffic_data.len() > 1 {
        let total_up: u64 = traffic_data.iter().map(|t| t.up).sum();
        let total_down: u64 = traffic_data.iter().map(|t| t.down).sum();
        let avg_up = total_up / traffic_data.len() as u64;
        let avg_down = total_down / traffic_data.len() as u64;

        println!(
            "📊 Average: ⬆️ {}/s ⬇️ {}/s",
            format_bytes(avg_up),
            format_bytes(avg_down)
        );
    }

    println!("\n📈 Testing real-time stream processing");

    let mut sample_count = 0;
    stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(3))
        .process_lines(|line| {
            if line.trim().is_empty() {
                return Ok(());
            }

            if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                sample_count += 1;
                println!(
                    "⚡ Live #{}: {} total/s",
                    sample_count,
                    format_bytes(traffic.up + traffic.down)
                );
            }

            if sample_count >= 5 {
                Err("Reached limit".into())
            } else {
                Ok(())
            }
        })
        .await
        .or_else(|e| {
            if e.to_string().contains("Reached limit") {
                Ok(())
            } else {
                Err(e)
            }
        })?;

    // Show pool stats
    if let Some(stats) = http_client.pool_stats() {
        println!("\n📊 HTTP Client Pool Stats: {}", stats);
    }

    println!("\n🎯 Architecture Comparison:");
    println!("┌─────────────────┬─────────────────────┬─────────────────────┐");
    println!("│ Feature         │ IpcHttpClient       │ IpcStreamClient     │");
    println!("├─────────────────┼─────────────────────┼─────────────────────┤");
    println!("│ Use Case        │ API calls, configs  │ Real-time monitoring│");
    println!("│ Response Type   │ Complete HTTP resp  │ Streaming data      │");
    println!("│ Connection Pool │ ✅ Built-in         │ ❌ Direct connects  │");
    println!("│ JSON Parsing    │ ✅ Type-safe        │ ✅ Stream-optimized │");
    println!("│ Timeout Control │ ✅ Per-request      │ ✅ Per-stream       │");
    println!("│ Error Handling  │ ✅ Rich status info │ ✅ Stream-aware     │");
    println!("│ Best For        │ GET, POST, PUT, etc │ Logs, metrics, events│");
    println!("└─────────────────┴─────────────────────┴─────────────────────┘");

    println!("\n🎉 Both clients work together seamlessly!");
    println!("📌 Use IpcHttpClient for: Configuration, API calls, one-time requests");
    println!("📌 Use IpcStreamClient for: Monitoring, logs, real-time data streams");

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0;

    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{:.0}{}", value, UNITS[unit_idx])
    } else {
        format!("{:.1}{}", value, UNITS[unit_idx])
    }
}
