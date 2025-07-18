use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcHttpClient, IpcStreamClient};
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
    // 1. 普通 HTTP 客户端 - 用于请求/响应
    println!("🔧 Testing IpcHttpClient (Request/Response)");
    let http_client = IpcHttpClient::new(&ipc_path)?;

    // 获取基本信息
    let proxies = http_client.get("/proxies").await?;
    println!("✅ Status: {}", proxies.status);
    println!("📄 Response length: {} chars", proxies.body.len());

    // 解析 JSON 响应
    if let Ok(json_data) = proxies.json() {
        if let Some(proxies_obj) = json_data.as_object() {
            println!("🔍 Found {} proxy groups", proxies_obj.len());
        }
    }

    println!("\n🌊 Testing IpcStreamClient (Streaming)");
    let stream_client = IpcStreamClient::new(&ipc_path)?;
    // 流式监控
    let traffic_data = stream_client
        .monitor_traffic(Duration::from_secs(6))
        .await?;

    println!("✅ Collected {} traffic samples", traffic_data.len());

    if !traffic_data.is_empty() {
        let latest = &traffic_data[traffic_data.len() - 1];
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
