use dotenv::dotenv;
use kode_bridge::{Result, IpcStreamClient, StreamClientConfig};
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

/// Extension trait for convenient traffic monitoring
pub trait TrafficMonitorExt {
    fn monitor_traffic(&self, timeout: Duration) -> impl std::future::Future<Output = Result<Vec<TrafficData>>> + Send;
    fn monitor_connections(&self, timeout: Duration) -> impl std::future::Future<Output = Result<Vec<ConnectionData>>> + Send;
}

impl TrafficMonitorExt for IpcStreamClient {
    async fn monitor_traffic(&self, timeout: Duration) -> Result<Vec<TrafficData>> {
        self.get("/traffic")
            .timeout(timeout)
            .json_results()
            .await
    }
    
    async fn monitor_connections(&self, timeout: Duration) -> Result<Vec<ConnectionData>> {
        self.get("/connections")
            .timeout(timeout)
            .json_results()
            .await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Traffic Monitoring Demo");
    println!("==========================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    // Configure streaming client for traffic monitoring
    let config = StreamClientConfig {
        default_timeout: Duration::from_secs(30),
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
        buffer_size: 32768, // Large buffer for high-frequency traffic data
    };

    let stream_client = IpcStreamClient::with_config(&ipc_path, config)?;

    println!("📊 Method 1: Batch traffic monitoring");

    let traffic_data = stream_client
        .monitor_traffic(Duration::from_secs(8))
        .await?;

    let total_samples = traffic_data.len();
    let total_up: u64 = traffic_data.iter().map(|t| t.up).sum();
    let total_down: u64 = traffic_data.iter().map(|t| t.down).sum();

    println!("✅ Collected {} traffic samples", total_samples);
    println!("📤 Total upload: {}", format_bytes(total_up));
    println!("📥 Total download: {}", format_bytes(total_down));

    if total_samples > 0 {
        let avg_up = total_up / total_samples as u64;
        let avg_down = total_down / total_samples as u64;
        println!(
            "📊 Average: ⬆️ {}/s ⬇️ {}/s",
            format_bytes(avg_up),
            format_bytes(avg_down)
        );
    }

    println!("\n📊 Method 2: Real-time stream processing");

    let mut sample_count = 0;
    let mut total_processed = 0u64;

    stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(5))
        .process_lines(|line| {
            if line.trim().is_empty() {
                return Ok(());
            }

            if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                sample_count += 1;
                total_processed += traffic.up + traffic.down;

                if sample_count % 3 == 0 {
                    println!(
                        "⚡ Sample #{}: {} total bytes/s",
                        sample_count,
                        format_bytes(traffic.up + traffic.down)
                    );
                }
            }

            if sample_count >= 10 {
                Err("Reached sample limit".into())
            } else {
                Ok(())
            }
        })
        .await
        .or_else(|e| {
            if e.to_string().contains("Reached sample limit") {
                Ok(())
            } else {
                Err(e)
            }
        })?;

    println!("✅ Real-time processing complete!");
    println!(
        "📊 Processed {} samples, {} total bytes",
        sample_count,
        format_bytes(total_processed)
    );

    println!("\n📊 Method 3: Connection monitoring");

    let connections = stream_client
        .monitor_connections(Duration::from_secs(3))
        .await?;

    println!("✅ Found {} active connections", connections.len());
    for (i, conn) in connections.iter().take(3).enumerate() {
        println!(
            "  {}. {} -> {} (↑{} ↓{})",
            i + 1,
            &conn.id[..8.min(conn.id.len())],
            conn.rule,
            format_bytes(conn.upload),
            format_bytes(conn.download)
        );
    }

    println!("\n📊 Method 4: Custom streaming with aggregation");

    let aggregated_stats = stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(4))
        .send()
        .await?
        .process_json(Duration::from_secs(4), |line| {
            if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                // Only keep samples with significant traffic
                let total = traffic.up + traffic.down;
                if total > 1024 { // More than 1KB/s
                    Some(total)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .await?;

    if !aggregated_stats.is_empty() {
        let total_traffic: u64 = aggregated_stats.iter().sum();
        let avg_traffic = total_traffic / aggregated_stats.len() as u64;
        let max_traffic = *aggregated_stats.iter().max().unwrap_or(&0);
        
        println!("📊 Traffic Statistics (samples with >1KB/s):");
        println!("  • Samples: {}", aggregated_stats.len());
        println!("  • Total: {}", format_bytes(total_traffic));
        println!("  • Average: {}/s", format_bytes(avg_traffic));
        println!("  • Peak: {}/s", format_bytes(max_traffic));
    }

    println!("\n🎯 Benefits of the traffic monitoring API:");
    println!("📌 Extension traits: custom convenience methods");
    println!("📌 Flexible collection: batch or real-time processing");
    println!("📌 Type-safe streaming: automatic JSON deserialization");
    println!("📌 Performance optimized: large buffers for high-frequency data");
    println!("📌 Statistics ready: easy aggregation and analysis");
    println!("📌 Error resilient: graceful handling of malformed data");

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