use dotenvy::dotenv;
use kode_bridge::{ClientConfig, IpcHttpClient, Result};
use serde_json::json;
use std::env;
use std::time::Duration;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ProxyGroup {
    pub name: String,
    pub r#type: String,
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ConfigUpdate {
    pub key: String,
    pub value: serde_json::Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    println!("🚀 Elegant HTTP Client Demo");
    println!("===========================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/example.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\example".to_string());

    // Create client with custom configuration
    let config = ClientConfig {
        default_timeout: Duration::from_secs(30),
        enable_pooling: true,
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
        ..Default::default()
    };

    let client = IpcHttpClient::with_config(&ipc_path, config)?;

    println!("📊 Method 1: Modern GET request with fluent API");

    // 🎯 Most elegant way: use like reqwest
    let response = client
        .get("/proxies")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    println!("✅ Response status: {}", response.status());
    println!("📄 Response length: {} bytes", response.content_length());
    println!("✨ Is success: {}", response.is_success());

    if response.is_success() {
        match response.json_value() {
            Ok(proxies) => {
                if let Some(proxies_obj) = proxies.as_object() {
                    println!("🔍 Found {} proxy groups", proxies_obj.len());

                    // Show first 3 proxy groups
                    for (count, (name, info)) in proxies_obj.iter().enumerate() {
                        if count >= 3 {
                            break;
                        }
                        println!(
                            "🔗 Proxy group: {} -> {}",
                            name,
                            info.get("type").unwrap_or(&serde_json::Value::Null)
                        );
                    }
                }
            }
            Err(e) => println!("⚠️ JSON parse error: {}", e),
        }
    }

    println!("\n📊 Method 2: Type-safe JSON handling");

    // 🎯 Type-safe JSON result
    let response = client
        .get("/configs")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    if response.is_success() {
        match response.json::<serde_json::Value>() {
            Ok(config) => {
                println!(
                    "✅ Config keys: {:?}",
                    config.as_object().map(|o| o.keys().collect::<Vec<_>>())
                );
            }
            Err(e) => println!("⚠️ JSON parse error: {}", e),
        }
    }

    println!("\n📊 Method 3: POST request with JSON body");

    // 🎯 POST request with elegant JSON handling
    let update_data = ConfigUpdate {
        key: "allow-lan".to_string(),
        value: serde_json::Value::Bool(true),
    };

    let response = client
        .post("/configs")
        .json_body(&json!(update_data))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ POST response status: {}", response.status());
    if response.is_success() {
        println!("✨ Configuration updated successfully!");
    } else if response.is_client_error() {
        println!("❌ Client error: {:?}", response.body());
    } else if response.is_server_error() {
        println!("💥 Server error: {:?}", response.body());
    }

    println!("\n📊 Method 4: PUT request with manual JSON");

    // 🎯 PUT request with manual JSON construction
    let proxy_config = json!({
        "name": "DIRECT",
        "type": "direct",
        "udp": true
    });

    let response = client
        .put("/proxies/DIRECT")
        .json_body(&proxy_config)
        .send()
        .await?;

    println!("✅ PUT response status: {}", response.status());

    println!("\n📊 Method 5: Error handling demonstration");

    // 🎯 Error handling demonstration
    let response = client
        .get("/non-existent-endpoint")
        .timeout(Duration::from_secs(2))
        .send()
        .await?;

    match response.status() {
        200..=299 => println!("✅ Success: {:?}", response.body()),
        400..=499 => println!(
            "❌ Client error {}: {:?}",
            response.status(),
            response.body()
        ),
        500..=599 => println!(
            "💥 Server error {}: {:?}",
            response.status(),
            response.body()
        ),
        _ => println!(
            "🤷 Unknown status {}: {:?}",
            response.status(),
            response.body()
        ),
    }

    println!("\n📊 Method 6: DELETE request");

    // 🎯 DELETE request
    let response = client
        .delete("/proxies/test-proxy")
        .timeout(Duration::from_secs(3))
        .send()
        .await?;

    println!("✅ DELETE response status: {}", response.status());

    println!("\n📊 Method 7: HEAD request");

    // 🎯 HEAD request
    let response = client
        .head("/")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ HEAD response status: {}", response.status());

    println!("\n📊 Method 8: Response inspection");

    // 🎯 Response inspection
    let response = client.get("/version").send().await?;

    println!("✅ Headers: {:?}", response.headers());
    println!("✅ Status: {}", response.status());
    println!("✅ Content length: {}", response.content_length());
    println!("✅ Is success: {}", response.is_success());
    println!("✅ Is client error: {}", response.is_client_error());
    println!("✅ Is server error: {}", response.is_server_error());

    if response.is_success() {
        match response.json_value() {
            Ok(version) => println!("🎉 Version info: {}", version),
            Err(e) => println!("⚠️ JSON parse error: {}", e),
        }
    }

    println!("\n📊 Method 9: Backward compatibility");

    // 🎯 Backward compatible methods are still available
    let response = client.request("GET", "/proxies", None).await?;
    println!("✅ Backward compatible GET status: {}", response.status);

    // Show pool stats
    if let Some(stats) = client.pool_stats() {
        println!("\n📊 Connection Pool Stats: {}", stats);
    }

    println!("\n🎯 Benefits of the new HTTP client API:");
    println!("📌 HTTP-like methods: get(), post(), put(), delete(), patch(), head()");
    println!("📌 Method chaining: .json_body().timeout().send()");
    println!("📌 Type-safe JSON: json<T>(), json_value()");
    println!("📌 Rich response handling: is_success(), is_client_error(), content_length()");
    println!("📌 Fluent API: .json_body().timeout().send()");
    println!("📌 Error categorization: is_client_error(), is_server_error()");
    println!("📌 Connection pooling: Built-in connection pool with statistics");
    println!("📌 Backward compatible: old methods still work");

    Ok(())
}
