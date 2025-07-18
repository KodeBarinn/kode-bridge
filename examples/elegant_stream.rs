use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcStreamClient};
use std::env;
use std::time::Duration;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ConnectionData {
    pub id: String,
    pub metadata: serde_json::Value,
    pub upload: u64,
    pub download: u64,
    pub start: String,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    println!("🚀 Elegant Stream Client Demo");
    println!("=============================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;

    let client = IpcStreamClient::with_timeout(ipc_path, Duration::from_secs(10))?;

    println!("📊 Method 1: HTTP-like GET request");

    let traffic_data: Vec<TrafficData> = client
        .get("/traffic")
        .timeout(Duration::from_secs(8))
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

    println!("\n📊 Method 2: Real-time processing with fluent API");

    let mut count = 0;
    client
        .get("/traffic")
        .timeout(Duration::from_secs(5))
        .process_lines(|line| {
            if line.trim().is_empty() {
                return true;
            }

            if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                count += 1;
                if count <= 3 {
                    println!(
                        "⚡ Traffic #{}: {} bytes/s total",
                        count,
                        format_bytes(traffic.up + traffic.down)
                    );
                }
            }

            count < 5
        })
        .await?;

    println!("\n📊 Method 4: POST request with JSON body");

    println!("✅ All methods demonstrated successfully!");

    println!("\n🎯 Benefits of the new API:");
    println!("📌 HTTP-like methods: get(), post(), put(), delete()");
    println!("📌 Method chaining: .timeout().json_results()");
    println!("📌 Type-safe JSON: automatically deserialize to your structs");
    println!("📌 Flexible response handling: take(), lines(), process_lines()");
    println!("📌 Backward compatible: old methods still work");

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
