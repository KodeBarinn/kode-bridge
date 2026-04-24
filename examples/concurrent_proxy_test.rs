use dotenvy::dotenv;
use kode_bridge::{ClientConfig, IpcHttpClient, KodeBridgeError, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateProxyPayload {
    pub name: String,
}

/// 并发代理测试和延迟测量
async fn concurrent_proxy_test(client: Arc<IpcHttpClient>, proxy_group: &str, proxies: Vec<&str>) -> Result<()> {
    println!("🚀 Starting concurrent proxy testing for group: {}", proxy_group);
    println!("📊 Testing {} proxies concurrently", proxies.len());

    let start_time = Instant::now();
    let mut tasks = Vec::new();

    // 创建并发任务
    for (index, proxy_name) in proxies.iter().enumerate() {
        let client = Arc::clone(&client);
        let proxy_group = proxy_group.to_string();
        let proxy_name = proxy_name.to_string();

        let task = tokio::spawn(async move {
            let task_start = Instant::now();
            println!("   🔄 Task {}: Testing proxy '{}'", index + 1, proxy_name);

            // 执行代理切换测试
            let result = test_proxy_switch(&client, &proxy_group, &proxy_name).await;
            let duration = task_start.elapsed();

            let success_status = match &result {
                Ok(success) => {
                    if *success {
                        println!(
                            "   ✅ Task {}: Proxy '{}' switched successfully in {:?}",
                            index + 1,
                            proxy_name,
                            duration
                        );
                        "success"
                    } else {
                        println!(
                            "   ⚠️ Task {}: Proxy '{}' switch failed in {:?}",
                            index + 1,
                            proxy_name,
                            duration
                        );
                        "failed"
                    }
                }
                Err(e) => {
                    println!(
                        "   ❌ Task {}: Proxy '{}' error in {:?}: {}",
                        index + 1,
                        proxy_name,
                        duration,
                        e
                    );
                    "error"
                }
            };

            (index, proxy_name, duration, success_status, result.is_ok())
        });

        tasks.push(task);
    }

    // 等待所有任务完成
    let mut results = Vec::new();
    for task in tasks {
        match task.await {
            Ok(result) => results.push(result),
            Err(e) => println!("   ❌ Task join error: {}", e),
        }
    }

    // 统计结果
    let total_duration = start_time.elapsed();
    let successful_count = results
        .iter()
        .filter(|(_, _, _, status, _)| *status == "success")
        .count();
    let failed_count = results.iter().filter(|(_, _, _, _, ok)| !ok).count();
    let timeout_count = results
        .iter()
        .filter(|(_, _, _, status, _)| *status == "error")
        .count();

    println!();
    println!("📊 Concurrent Test Results:");
    println!("   🎯 Total proxies tested: {}", proxies.len());
    println!("   ✅ Successful switches: {}", successful_count);
    println!("   ❌ Failed requests: {}", failed_count);
    println!("   ⏰ Timeout errors: {}", timeout_count);
    println!("   🕐 Total time: {:?}", total_duration);
    println!(
        "   📈 Average time per request: {:?}",
        Duration::from_millis(total_duration.as_millis() as u64 / proxies.len() as u64)
    );

    // 分析延迟分布
    let mut durations: Vec<Duration> = results
        .iter()
        .map(|(_, _, duration, _, _)| *duration)
        .collect();
    durations.sort();

    if !durations.is_empty() {
        let min_duration = durations[0];
        let max_duration = durations[durations.len() - 1];
        let median_duration = durations[durations.len() / 2];

        println!("   📊 Latency Analysis:");
        println!("      ⚡ Fastest: {:?}", min_duration);
        println!("      🐌 Slowest: {:?}", max_duration);
        println!("      📊 Median: {:?}", median_duration);
    }

    // 如果超时率过高，给出建议
    let timeout_rate = timeout_count as f64 / proxies.len() as f64;
    if timeout_rate > 0.3 {
        println!();
        println!("⚠️ High timeout rate detected ({:.1}%)", timeout_rate * 100.0);
        println!("💡 Suggestions:");
        println!("   - Reduce concurrent requests (current: {})", proxies.len());
        println!("   - Increase timeout duration");
        println!("   - Check server load and network conditions");
    }

    Ok(())
}

/// 测试单个代理切换
async fn test_proxy_switch(client: &IpcHttpClient, proxy_group: &str, proxy_name: &str) -> Result<bool> {
    let path = format!("/proxies/{}", proxy_group);
    let payload = UpdateProxyPayload {
        name: proxy_name.to_string(),
    };

    // 执行代理切换请求
    let response = client
        .put(&path)
        .json_body(&serde_json::to_value(&payload)?)
        .timeout(Duration::from_secs(8)) // 设置较短的超时
        .send()
        .await?;

    if response.is_success() {
        // 验证切换是否生效
        tokio::time::sleep(Duration::from_millis(500)).await;
        verify_proxy_switch(client, proxy_group, proxy_name).await
    } else {
        Ok(false)
    }
}

/// 验证代理切换是否生效
async fn verify_proxy_switch(client: &IpcHttpClient, proxy_group: &str, expected_proxy: &str) -> Result<bool> {
    let response = client
        .get("/proxies")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    if !response.is_success() {
        return Ok(false);
    }

    let proxy_data = response.json_value()?;

    if let Some(proxies_obj) = proxy_data.get("proxies").and_then(|v| v.as_object()) {
        if let Some(group_info) = proxies_obj.get(proxy_group) {
            if let Some(current_proxy) = group_info.get("now").and_then(|v| v.as_str()) {
                return Ok(current_proxy == expected_proxy);
            }
        }
    }

    Ok(false)
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🧪 Concurrent Proxy Testing Example");
    println!("=====================================");

    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());

    // 配置优化的HTTP客户端
    let config = ClientConfig {
        default_timeout: Duration::from_secs(8),
        enable_pooling: true,
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
        max_concurrent_requests: 8,         // 限制并发数
        max_requests_per_second: Some(5.0), // 限制请求频率
        ..Default::default()
    };

    let client = Arc::new(IpcHttpClient::with_config(&ipc_path, config)?);

    // 获取可用代理
    println!("📡 Fetching available proxies...");
    let response = client
        .get("/proxies")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !response.is_success() {
        return Err(KodeBridgeError::connection(format!(
            "Failed to get proxies: {}",
            response.status()
        )));
    }

    let proxy_data = response.json_value()?;

    if let Some(proxies_obj) = proxy_data.get("proxies").and_then(|v| v.as_object()) {
        // 寻找第一个有多个代理的组进行测试
        for (group_name, group_info) in proxies_obj.iter() {
            if let Some(all_proxies) = group_info.get("all").and_then(|v| v.as_array()) {
                if all_proxies.len() >= 3 {
                    // 至少3个代理才进行并发测试
                    let proxy_names: Vec<&str> = all_proxies
                        .iter()
                        .filter_map(|v| v.as_str())
                        .take(8) // 限制最多8个并发
                        .collect();

                    if !proxy_names.is_empty() {
                        println!(
                            "🎯 Found group '{}' with {} proxies for testing",
                            group_name,
                            proxy_names.len()
                        );

                        // 执行并发测试
                        if let Err(e) = concurrent_proxy_test(Arc::clone(&client), group_name, proxy_names).await {
                            println!("❌ Concurrent test failed: {}", e);
                        }

                        // 显示连接池统计
                        if let Some(stats) = client.pool_stats() {
                            println!();
                            println!("🔗 Connection Pool Stats: {}", stats);
                        }

                        break;
                    }
                }
            }
        }
    } else {
        println!("⚠️ No suitable proxy groups found for testing");
    }

    // 清理资源
    client.close();
    drop(client);

    println!();
    println!("🎯 Concurrent testing completed!");
    println!("💡 Key optimizations applied:");
    println!("   ✅ Connection pooling with increased size (20 connections)");
    println!("   ✅ Reduced timeouts (8s request, 5s connection)");
    println!("   ✅ Enhanced retry logic with exponential backoff");
    println!("   ✅ Concurrent request limiting (8 max)");
    println!("   ✅ Rate limiting (5 requests/sec)");
    println!("   ✅ Jitter in retry delays to prevent thundering herd");

    Ok(())
}
