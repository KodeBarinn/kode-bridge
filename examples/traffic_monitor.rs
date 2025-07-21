use dotenv::dotenv;
use kode_bridge::{IpcStreamClient, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{RwLock, watch};
use tokio::time::Duration;

// Traffic data structure - same as in two_clients.rs
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

// Current traffic rate structure
#[derive(Debug, Clone)]
pub struct CurrentTraffic {
    pub up_rate: u64,    // bytes per second
    pub down_rate: u64,  // bytes per second
    pub total_up: u64,   // total uploaded bytes
    pub total_down: u64, // total downloaded bytes
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

// Traffic monitor for continuous monitoring - ULTRA SIMPLIFIED
pub struct TrafficMonitor {
    current: Arc<RwLock<CurrentTraffic>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TrafficMonitor {
    pub fn new(client: IpcStreamClient) -> Self {
        let current = Arc::new(RwLock::new(CurrentTraffic::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        
        let monitor_current = current.clone();
        let handle = tokio::spawn(async move {
            Self::continuous_monitor(client, monitor_current, shutdown_rx).await;
        });

        Self {
            current,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    // COMPLETELY SIMPLIFIED: copy the exact pattern from traffic.rs
    async fn continuous_monitor(
        client: IpcStreamClient,
        current: Arc<RwLock<CurrentTraffic>>,
        shutdown_rx: watch::Receiver<bool>,
    ) {
        let mut last_traffic: Option<TrafficData> = None;
        let mut sample_count = 0;
        
        loop {
            // Check for shutdown
            if shutdown_rx.has_changed().unwrap_or(false) && *shutdown_rx.borrow() {
                println!("🛑 Traffic monitor shutting down...");
                break;
            }

            // Direct streaming like traffic.rs - no complications
            let result = client
                .get("/traffic")
                .timeout(Duration::from_secs(5))
                .process_lines(|line| {
                    if line.trim().is_empty() {
                        return Ok(());
                    }

                    if let Ok(traffic) = serde_json::from_str::<TrafficData>(line) {
                        sample_count += 1;
                        
                        // Update every 1 samples
                        if sample_count % 1 == 0 {
                            let now = Instant::now();
                            
                            let (up_rate, down_rate) = if let Some(last) = &last_traffic {
                                let up_diff = traffic.up.saturating_sub(last.up);
                                let down_diff = traffic.down.saturating_sub(last.down);
                                (up_diff, down_diff)
                            } else {
                                (0, 0)
                            };

                            // Update directly in the current task context
                            let current_clone = current.clone();
                            tokio::spawn(async move {
                                let mut current_traffic = current_clone.write().await;
                                current_traffic.up_rate = up_rate;
                                current_traffic.down_rate = down_rate;
                                current_traffic.total_up = traffic.up;
                                current_traffic.total_down = traffic.down;
                                current_traffic.last_updated = now;
                            });
                            
                            last_traffic = Some(traffic.clone());
                            println!("🔄 Sample #{}: ⬆️ {} ⬇️ {}", sample_count, traffic.up, traffic.down);
                        } else {
                            last_traffic = Some(traffic.clone());
                        }
                        
                        // Stop after 15 samples, then restart the stream
                        if sample_count >= 15 {
                            return Err("Restart stream".into());
                        }
                    }
                    Ok(())
                })
                .await;
                
            match result {
                Err(e) if e.to_string().contains("Restart stream") => {
                    // Normal restart - reset counter
                    sample_count = 0;
                    continue;
                }
                Err(e) => {
                    eprintln!("⚠️ Stream error: {}, retrying in 2s...", e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Ok(_) => {
                    // Stream ended normally, restart
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    // Get current traffic data
    pub async fn current(&self) -> CurrentTraffic {
        self.current.read().await.clone()
    }

    // Get current upload rate
    pub async fn up_rate(&self) -> u64 {
        self.current.read().await.up_rate
    }

    // Get current download rate  
    pub async fn down_rate(&self) -> u64 {
        self.current.read().await.down_rate
    }

    // Get total traffic
    pub async fn totals(&self) -> (u64, u64) {
        let current = self.current.read().await;
        (current.total_up, current.total_down)
    }

    // Check if data is recent (within last 5 seconds)
    pub async fn is_data_fresh(&self) -> bool {
        let current = self.current.read().await;
        current.last_updated.elapsed() < Duration::from_secs(5)
    }

    // Stop monitoring
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for TrafficMonitor {
    fn drop(&mut self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(true);
        }
    }
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

// Usage example in main function
async fn example_usage() -> Result<()> {
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    println!("🔧 Connecting to: {}", ipc_path);

    // Test connection first
    let stream_client = IpcStreamClient::new(&ipc_path)?;
    
    // Quick connection test
    println!("🔍 Testing connection...");
    match tokio::time::timeout(Duration::from_secs(3), test_connection(&stream_client)).await {
        Ok(Ok(())) => println!("✅ Connection test successful"),
        Ok(Err(e)) => {
            println!("❌ Connection test failed: {}", e);
            println!("💡 Make sure the server is running and the socket path is correct");
            return Ok(());
        }
        Err(_) => {
            println!("❌ Connection test timed out");
            println!("💡 Make sure the server is running and accessible");
            return Ok(());
        }
    }
    
    // Start traffic monitoring
    let mut monitor = TrafficMonitor::new(stream_client);
    
    // Wait a bit for initial data
    println!("⏳ Waiting for initial data...");
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Get current traffic info
    for i in 0..10 {
        let current = monitor.current().await;
        println!(
            "📊 #{}: ⬆️ {}/s ⬇️ {}/s (Total: ⬆️ {} ⬇️ {})",
            i + 1,
            format_bytes(current.up_rate),
            format_bytes(current.down_rate),
            format_bytes(current.total_up),
            format_bytes(current.total_down)
        );
        
        if !monitor.is_data_fresh().await {
            println!("⚠️ Data might be stale");
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    // Stop monitoring
    monitor.stop().await;
    
    Ok(())
}

async fn test_connection(client: &IpcStreamClient) -> Result<()> {
    // Simple connection test: try to get any response
    let test_result = client
        .get("/traffic")
        .timeout(Duration::from_secs(2))
        .process_lines(|_line| {
            // Got a response, connection works
            Err("Connection OK".into())
        })
        .await;

    match test_result {
        Err(e) if e.to_string().contains("Connection OK") => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "No response").into()),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Traffic Monitor Example");
    println!("=========================");

    example_usage().await?;

    Ok(())
}