use async_trait::async_trait;
use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcHttpClient, IpcStreamClient};

// Traffic data structure
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

// Extension trait
#[async_trait]
pub trait MyStreamClientExt {
    async fn monitor_traffic(&self, timeout: Duration) -> AnyResult<Vec<TrafficData>>;
}

#[async_trait]
impl MyStreamClientExt for IpcStreamClient {
    async fn monitor_traffic(&self, timeout: Duration) -> AnyResult<Vec<TrafficData>> {
        self.get_json_stream("/traffic", timeout).await
    }
}
use std::time::Duration;

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    println!("🏗️  Clean Architecture: Two Client Types");
    println!("========================================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;
    // 1. Regular HTTP client - for request/response
    println!("🔧 Testing IpcHttpClient (Request/Response)");
    let http_client = IpcHttpClient::new(&ipc_path)?;

    // Get basic information - using new elegant API
    let response = http_client
        .get("/proxies")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ Status: {}", response.status());
    println!("📄 Response length: {} bytes", response.content_length());
    println!("✨ Is success: {}", response.is_success());

    // Parse JSON response
    if response.is_success() {
        let json_data = response.json_value()?;
        if let Some(proxies_obj) = json_data.as_object() {
            println!("🔍 Found {} proxy groups", proxies_obj.len());
        }
    }

    println!("\n🌊 Testing IpcStreamClient (Streaming)");
    let stream_client = IpcStreamClient::new(&ipc_path)?;

    // Stream monitoring - using new elegant API
    let traffic_data: Vec<TrafficData> = stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(6))
        .json_results()
        .await?;

    println!("✅ Collected {} traffic samples", traffic_data.len());

    if let Some(latest) = traffic_data.last() {
        println!(
            "📊 Latest: ⬆️ {} ⬇️ {}",
            format_bytes(latest.up),
            format_bytes(latest.down)
        );
    }

    println!("\n🎯 Comparing the two approaches:");
    println!("📌 IpcHttpClient: Best for API calls, configuration, one-time queries");
    println!("📌 IpcStreamClient: Best for real-time monitoring, continuous data");

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
