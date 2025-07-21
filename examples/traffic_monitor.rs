use dotenv::dotenv;
use kode_bridge::{IpcStreamClient, Result};
use serde::{Deserialize, Serialize};
use std::{env, sync::Arc, time::Instant};
use tokio::{sync::RwLock, time::Duration};

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

// Minimal traffic monitor
pub struct TrafficMonitor(Arc<RwLock<CurrentTraffic>>);

impl TrafficMonitor {
    pub fn new(client: IpcStreamClient) -> Self {
        let current = Arc::new(RwLock::new(CurrentTraffic::default()));
        let monitor_current = current.clone();

        tokio::spawn(async move {
            let mut last: Option<TrafficData> = None;
            loop {
                let _ = client
                    .get("/traffic")
                    .timeout(Duration::from_secs(10))
                    .process_lines(|line| {
                        if let Ok(traffic) = serde_json::from_str::<TrafficData>(line.trim()) {
                            let (up_rate, down_rate) = last
                                .as_ref()
                                .map(|l| {
                                    (
                                        traffic.up.saturating_sub(l.up),
                                        traffic.down.saturating_sub(l.down),
                                    )
                                })
                                .unwrap_or((0, 0));

                            tokio::spawn({
                                let current = monitor_current.clone();
                                async move {
                                    *current.write().await = CurrentTraffic {
                                        up_rate,
                                        down_rate,
                                        total_up: traffic.up,
                                        total_down: traffic.down,
                                        last_updated: Instant::now(),
                                    };
                                }
                            });
                            last = Some(traffic);
                        }
                        Ok(())
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        Self(current)
    }

    pub async fn current(&self) -> CurrentTraffic {
        self.0.read().await.clone()
    }
    pub async fn is_fresh(&self) -> bool {
        self.0.read().await.last_updated.elapsed() < Duration::from_secs(5)
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let (mut val, mut unit) = (bytes as f64, 0);
    while val >= 1024.0 && unit < 3 {
        val /= 1024.0;
        unit += 1;
    }
    format!("{:.1}{}", val, UNITS[unit])
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    let client =
        IpcStreamClient::new(&env::var("CUSTOM_SOCK").unwrap_or("/tmp/example.sock".into()))?;

    // Test connection
    if client
        .get("/traffic")
        .timeout(Duration::from_secs(2))
        .process_lines(|_| Err("OK".into()))
        .await
        .is_ok()
    {
        println!("❌ No server");
        return Ok(());
    }

    println!("🚀 Traffic Monitor ✅");
    let monitor = TrafficMonitor::new(client);

    for i in 1..=10 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let d = monitor.current().await;
        println!(
            "📊 #{}: ⬆️{}/s ⬇️{}/s (⬆️{} ⬇️{}) {}",
            i,
            fmt_bytes(d.up_rate),
            fmt_bytes(d.down_rate),
            fmt_bytes(d.total_up),
            fmt_bytes(d.total_down),
            if monitor.is_fresh().await {
                "✅"
            } else {
                "⚠️"
            }
        );
    }
    Ok(())
}
