use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcHttpClient};
use serde::Deserialize;
use std::env;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct Traffic {
    up: u64,
    down: u64,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    dotenv().ok();

    // 使用环境变量或默认路径
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;

    let client = IpcHttpClient::new(&ipc_path)?;

    let response = client.request_stream("GET", "/traffic", None).await?;

    let traffic_data: Vec<Traffic> = response.json(Duration::from_millis(1)).await?;

    let stats = analyze_traffic(&traffic_data);
    println!("📈 统计结果: {}", stats);

    Ok(())
}

fn analyze_traffic(data: &[Traffic]) -> String {
    let total_up: u64 = data.iter().map(|t| t.up).sum();
    let total_down: u64 = data.iter().map(|t| t.down).sum();
    let samples = data.len();

    format!(
        "{}样本, 上传{}KB, 下载{}KB",
        samples,
        total_up / 1024,
        total_down / 1024
    )
}
