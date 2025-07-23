use dotenv::dotenv;
use kode_bridge::{IpcHttpClient, Result, ClientConfig, KodeBridgeError};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateProxyPayload {
    pub name: String,
}

async fn get_proxy_groups(client: &IpcHttpClient) -> Result<serde_json::Value> {
    println!("📡 Getting all proxy groups...");
    
    let response = client
        .get("/proxies")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    
    println!("✅ Status: {}", response.status());
    
    if response.is_success() {
        let proxy_data = response.json_value()?;
        
        if let Some(proxies_map) = proxy_data.get("proxies").and_then(|v| v.as_object()) {
            println!("🔍 Found {} proxy groups", proxies_map.len());
        }
        
        Ok(proxy_data)
    } else {
        Err(KodeBridgeError::connection(format!("Failed to get proxies: {}", response.status())))
    }
}

async fn update_proxy(client: &IpcHttpClient, proxy_group: &str, proxy_name: &str) -> Result<()> {
    let path = format!("/proxies/{}", proxy_group);
    let payload = UpdateProxyPayload {
        name: proxy_name.to_string(),
    };
    
    println!("🔄 Updating proxy...");
    println!("   Path: {}", path);
    println!("   Method: PUT");
    println!("   Payload: {:?}", payload);
    
    // 诊断：记录开始时间
    let start_time = std::time::Instant::now();
    println!("   🕐 Request started at: {:?}", start_time);
    
    // 增加超时时间到15秒，并添加详细的错误信息
    let result = client
        .put(&path)
        .json_body(&serde_json::to_value(&payload)?)
        .timeout(Duration::from_secs(15))
        .send()
        .await;
    
    let elapsed = start_time.elapsed();
    println!("   ⏱️ Total elapsed time: {:?}", elapsed);
    
    match result {
        Ok(response) => {
            println!("   ✅ Response received successfully");
            println!("   📊 Response status: {}", response.status());
            
            if let Ok(body_text) = response.body() {
                println!("   📄 Response body: {}", body_text);
            }
            
            if response.is_success() {
                println!("✅ Proxy updated successfully");
                Ok(())
            } else {
                Err(KodeBridgeError::connection(format!("Failed to update proxy: {}", response.status())))
            }
        }
        Err(e) => {
            println!("   ❌ Request failed after {:?}", elapsed);
            println!("   🔍 Error type: {:?}", e);
            
            // 检查是否是超时错误
            if let Some(err_msg) = format!("{}", e).strip_prefix("Timeout error:") {
                println!("   ⏰ This is a timeout error: {}", err_msg.trim());
                println!("   💡 Possible causes:");
                println!("      - Server is taking too long to respond");
                println!("      - Network connectivity issues");
                println!("      - Server is processing the request but not responding");
                println!("      - The proxy switch might still succeed despite timeout");
            }
            
            Err(e)
        }
    }
}

async fn force_refresh_proxies(client: &IpcHttpClient) -> Result<()> {
    println!("🔃 Force refreshing proxy cache...");
    
    // Some proxy APIs use different endpoints for cache refresh
    let endpoints = ["/proxies", "/providers/proxies"];
    
    for endpoint in &endpoints {
        let response = client
            .put(endpoint)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        
        match response {
            Ok(resp) if resp.is_success() => {
                println!("✅ Cache refreshed via {}", endpoint);
                return Ok(());
            }
            Ok(resp) => {
                println!("⚠️ {} returned: {}", endpoint, resp.status());
            }
            Err(e) => {
                println!("⚠️ {} failed: {}", endpoint, e);
            }
        }
    }
    
    println!("⚠️ Cache refresh might not be supported or failed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Proxy Management Example");
    println!("===========================");

    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    
    // Configure HTTP client
    let config = ClientConfig {
        default_timeout: Duration::from_secs(15),
        enable_pooling: true,
        max_retries: 2,
        retry_delay: Duration::from_millis(300),
        ..Default::default()
    };

    let client = IpcHttpClient::with_config(&ipc_path, config)?;
    
        // 1. Get all proxy groups
    match get_proxy_groups(&client).await {
        Ok(proxy_data) => {
            // Display proxy groups
            if let Some(proxies_obj) = proxy_data.get("proxies").and_then(|v| v.as_object()) {
                for (count, (group_name, proxy_info)) in proxies_obj.iter().enumerate() {
                    if count >= 3 { break; }
                    
                    println!("📂 Group: {}", group_name);
                    
                    if let Some(proxy_type) = proxy_info.get("type") {
                        println!("   Type: {}", proxy_type.as_str().unwrap_or("Unknown"));
                    }
                    if let Some(current) = proxy_info.get("now") {
                        println!("   Current: {}", current.as_str().unwrap_or("None"));
                    }
                    if let Some(all_proxies) = proxy_info.get("all").and_then(|v| v.as_array()) {
                        println!("   Available: {} options", all_proxies.len());
                        
                        // Show first few available proxies
                        for (i, proxy) in all_proxies.iter().enumerate() {
                            if i >= 3 { break; }
                            if let Some(proxy_name) = proxy.as_str() {
                                println!("     - {}", proxy_name);
                            }
                        }
                        if all_proxies.len() > 3 {
                            println!("     ... and {} more", all_proxies.len() - 3);
                        }
                    }
                    println!();
                }
                
                // 2. Try to update a proxy (example with MyGroup group if available)
                if let Some(ssrdog_info) = proxies_obj.get("MyGroup") {
                    if let Some(all_proxies) = ssrdog_info.get("all").and_then(|v| v.as_array()) {
                        // 记录切换前的状态
                        if let Some(current_before) = ssrdog_info.get("now").and_then(|v| v.as_str()) {
                            println!("   � Current proxy BEFORE switch: {}", current_before);
                        }
                        
                        // 选择一个不同的代理进行切换测试
                        let target_proxy = if let Some(current) = ssrdog_info.get("now").and_then(|v| v.as_str()) {
                            // 找到一个与当前不同的代理
                            all_proxies.iter()
                                .find_map(|v| v.as_str())
                                .filter(|&proxy| proxy != current)
                                .unwrap_or(current)
                        } else {
                            all_proxies.first().and_then(|v| v.as_str()).unwrap_or("DIRECT")
                        };
                        
                        println!("🔄 Demonstrating proxy switch for MyGroup group...");
                        println!("   🎯 Target proxy: {}", target_proxy);
                        
                        if let Err(e) = update_proxy(&client, "MyGroup", target_proxy).await {
                            println!("⚠️ Update failed: {}", e);
                        }
                        
                        // 验证切换是否成功：等待一下然后重新获取代理状态
                        println!("   🔍 Verifying proxy switch after timeout...");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        
                        match get_proxy_groups(&client).await {
                            Ok(updated_data) => {
                                if let Some(updated_proxies) = updated_data.get("proxies").and_then(|v| v.as_object()) {
                                    if let Some(updated_ssrdog) = updated_proxies.get("MyGroup") {
                                        if let Some(current_after) = updated_ssrdog.get("now").and_then(|v| v.as_str()) {
                                            println!("   📍 Current proxy AFTER switch: {}", current_after);
                                            
                                            if current_after == target_proxy {
                                                println!("   ✅ Proxy switch WAS SUCCESSFUL despite timeout!");
                                                println!("   💡 The timeout is in the response, not the actual operation");
                                            } else {
                                                println!("   ❌ Proxy switch did not take effect");
                                                println!("   🔍 Expected: {}, Actual: {}", target_proxy, current_after);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                println!("   ⚠️ Failed to verify switch: {}", e);
                            }
                        }
                    }
                } else if let Some((group_name, group_info)) = proxies_obj.iter().next() {
                    // Fallback to first group if MyGroup not available
                    if let Some(all_proxies) = group_info.get("all").and_then(|v| v.as_array()) {
                        if let Some(first_proxy) = all_proxies.first().and_then(|v| v.as_str()) {
                            println!("🔄 Demonstrating proxy switch for {} group...", group_name);
                            if let Err(e) = update_proxy(&client, group_name, first_proxy).await {
                                println!("⚠️ Update failed: {}", e);
                            }
                        }
                    }
                }
            } else {
                println!("⚠️ Unexpected response format");
            }
        }
        Err(e) => {
            println!("❌ Failed to get proxies: {}", e);
        }
    }
    
    println!();
    
    // 3. Force refresh proxy cache
    if let Err(e) = force_refresh_proxies(&client).await {
        println!("⚠️ Cache refresh failed: {}", e);
    }
    
    println!();
    println!("🎯 Proxy management operations completed!");
    println!("💡 Features demonstrated:");
    println!("   ✅ List all proxy groups and their options");
    println!("   ✅ Update/switch proxy for a group");
    println!("   ✅ Force refresh proxy cache");
    println!("   ✅ Handle different response formats");
    println!("   ✅ Robust error handling");

    Ok(())
}
