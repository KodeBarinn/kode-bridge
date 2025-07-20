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

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Elegant Stream Client Demo");
    println!("=============================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    // Create client with custom streaming configuration
    let config = StreamClientConfig {
        default_timeout: Duration::from_secs(60),
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
        buffer_size: 16384, // Larger buffer for streaming
    };

    let client = IpcStreamClient::with_config(&ipc_path, config)?;

    println!("📊 Method 1: Collect JSON stream data");

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

    println!("\n📊 Method 2: Real-time stream processing");

    let mut count = 0;
    client
        .get("/traffic")
        .timeout(Duration::from_secs(5))
        .process_lines(|line| {
            if line.trim().is_empty() {
                return Ok(());
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

            if count >= 5 {
                // Signal to stop processing
                Err("Reached max count".into())
            } else {
                Ok(())
            }
        })
        .await
        .or_else(|e| {
            if e.to_string().contains("Reached max count") {
                Ok(())
            } else {
                Err(e)
            }
        })?;

    println!("\n📊 Method 3: Custom JSON processing with handler");

    let processed_data: Vec<(u64, String)> = client
        .get("/traffic")
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .process_json(Duration::from_secs(3), |line| {
            // Custom JSON processing logic
            if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                let total = traffic.up + traffic.down;
                if total > 0 {
                    Some((total, format_bytes(total)))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .await?;

    println!("✅ Processed {} valid traffic entries", processed_data.len());
    for (i, (bytes, formatted)) in processed_data.iter().take(3).enumerate() {
        println!("  {}. {} bytes ({})", i + 1, bytes, formatted);
    }

    println!("\n📊 Method 4: Stream with custom timeout");

    let text_data = client
        .get("/logs")
        .timeout(Duration::from_secs(2))
        .send()
        .await?
        .collect_text_with_timeout(Duration::from_secs(2))
        .await?;

    println!("✅ Collected {} characters of log data", text_data.len());
    if !text_data.is_empty() {
        let preview = if text_data.len() > 200 {
            format!("{}...", &text_data[..200])
        } else {
            text_data
        };
        println!("📄 Preview: {}", preview);
    }

    println!("\n📊 Method 5: Type-safe streaming with complex types");

    let connections: Vec<ConnectionData> = client
        .get("/connections")
        .timeout(Duration::from_secs(5))
        .json_results()
        .await?;

    println!("✅ Found {} active connections", connections.len());
    for (i, conn) in connections.iter().take(2).enumerate() {
        println!(
            "  {}. {} - {} ({} / {})",
            i + 1,
            conn.id,
            conn.rule,
            format_bytes(conn.upload),
            format_bytes(conn.download)
        );
    }

    println!("\n🎯 Benefits of the new streaming API:");
    println!("📌 Fluent interface: .get().timeout().json_results()");
    println!("📌 Type-safe JSON streaming: automatic deserialization");
    println!("📌 Flexible processing: json_results(), process_lines(), collect_text()");
    println!("📌 Timeout control: per-request and global timeouts");
    println!("📌 Error handling: graceful stream termination");
    println!("📌 High performance: configurable buffers and connection reuse");
    println!("📌 Real-time processing: process data as it arrives");

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