use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcStreamClient, TrafficData};
use std::time::Duration;

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();
    println!("🚀 Ultra Clean Architecture Demo");
    println!("===============================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;
    let stream_client = IpcStreamClient::new(ipc_path)?;

    println!("📊 Method 1: Direct traffic monitoring");

    // 🎯 超级简洁！专门的流量监控方法
    let traffic_data = stream_client
        .monitor_traffic(Duration::from_secs(8))
        .await?;

    // 分析数据
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

    // 实时处理演示
    let mut sample_count = 0;
    let mut total_processed = 0u64;

    stream_client
        .process_stream("/traffic", Duration::from_secs(5), |line| {
            if line.trim().is_empty() {
                return true;
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

            true // 继续处理
        })
        .await?;

    println!("✅ Real-time processing complete!");
    println!(
        "📊 Processed {} samples, {} total bytes",
        sample_count,
        format_bytes(total_processed)
    );

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
