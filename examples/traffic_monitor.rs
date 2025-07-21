use dotenv::dotenv;
use kode_bridge::{IpcStreamClient, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

#[derive(Debug, Clone)]
pub struct CurrentTraffic {
    pub up_rate: u64,
    pub down_rate: u64,
    pub total_up: u64,
    pub total_down: u64,
    pub last_updated: Instant,
}

impl Default for CurrentTraffic {
    fn default() -> Self {
        Self {
            up_rate: 0,
            down_rate: 0,
            total_up: 0,
            total_down: 0,
            last_updated: Instant::now(),
        }
    }
}

// Ultra-simplified traffic monitor
pub struct TrafficMonitor {
    current: Arc<RwLock<CurrentTraffic>>,
}

impl TrafficMonitor {
    pub fn new(client: IpcStreamClient) -> Self {
        let current = Arc::new(RwLock::new(CurrentTraffic::default()));
        let monitor_current = current.clone();
        
        tokio::spawn(async move {
            Self::monitor_loop(client, monitor_current).await;
        });

        Self { current }
    }

    async fn monitor_loop(client: IpcStreamClient, current: Arc<RwLock<CurrentTraffic>>) {
        let mut last_traffic: Option<TrafficData> = None;
        
        loop {
            // Simple continuous streaming
            let _ = client
                .get("/traffic")
                .timeout(Duration::from_secs(10))
                .process_lines(|line| {
                    if let Ok(traffic) = serde_json::from_str::<TrafficData>(line.trim()) {
                        let (up_rate, down_rate) = if let Some(last) = &last_traffic {
                            (
                                traffic.up.saturating_sub(last.up),
                                traffic.down.saturating_sub(last.down)
                            )
                        } else { (0, 0) };

                        // Non-blocking update
                        let current = current.clone();
                        tokio::spawn(async move {
                            let mut data = current.write().await;
                            *data = CurrentTraffic {
                                up_rate,
                                down_rate,
                                total_up: traffic.up,
                                total_down: traffic.down,
                                last_updated: Instant::now(),
                            };
                        });
                        
                        last_traffic = Some(traffic);
                    }
                    Ok(())
                })
                .await;
                
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn current(&self) -> CurrentTraffic {
        self.current.read().await.clone()
    }

    pub async fn is_data_fresh(&self) -> bool {
        self.current.read().await.last_updated.elapsed() < Duration::from_secs(5)
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    
    format!("{:.1}{}", value, UNITS[unit])
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Traffic Monitor");
    
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    let client = IpcStreamClient::new(&ipc_path)?;
    
    // Quick connection test
    let test_ok = client
        .get("/traffic")
        .timeout(Duration::from_secs(2))
        .process_lines(|_| Err("OK".into()))
        .await
        .is_err();
    
    if !test_ok {
        println!("❌ No server found");
        return Ok(());
    }
    
    println!("✅ Connected, starting monitor...");
    let monitor = TrafficMonitor::new(client);
    
    // Monitor for 10 seconds
    for i in 1..=10 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let data = monitor.current().await;
        
        println!(
            "📊 #{}: ⬆️ {}/s ⬇️ {}/s (Total: ⬆️ {} ⬇️ {}) {}",
            i,
            format_bytes(data.up_rate),
            format_bytes(data.down_rate),
            format_bytes(data.total_up),
            format_bytes(data.total_down),
            if monitor.is_data_fresh().await { "✅" } else { "⚠️" }
        );
    }
    
    Ok(())
}