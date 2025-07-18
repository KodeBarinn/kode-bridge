use dotenv::dotenv;
use kode_bridge::{AnyResult, IpcHttpClient};
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
async fn main() -> AnyResult<()> {
    dotenv().ok();
    println!("🚀 Elegant HTTP Client Demo");
    println!("===========================");

    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;

    // Create client with custom default timeout
    let client = IpcHttpClient::with_timeout(ipc_path, Duration::from_secs(30))?;

    println!("📊 Method 1: HTTP-like GET request");

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
        let proxies: serde_json::Value = response.json()?;
        if let Some(proxies_obj) = proxies.as_object() {
            println!("🔍 Found {} proxy groups", proxies_obj.len());

            // Show first 3 proxy groups
            for (count, (name, info)) in proxies_obj.into_iter().enumerate() {
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

    println!("\n📊 Method 2: Direct JSON result");

    // 🎯 Direct JSON result
    let config: serde_json::Value = client
        .get("/configs")
        .timeout(Duration::from_secs(5))
        .json_result()
        .await?;

    println!(
        "✅ Config keys: {:?}",
        config.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );

    println!("\n📊 Method 3: POST request with JSON body");

    // 🎯 POST request with elegant JSON handling
    let update_data = ConfigUpdate {
        key: "allow-lan".to_string(),
        value: serde_json::Value::Bool(true),
    };

    let response = client
        .post("/configs")
        .json_body(&update_data)?
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ POST response status: {}", response.status());
    if response.is_success() {
        println!("✨ Configuration updated successfully!");
    } else if response.is_client_error() {
        println!("❌ Client error: {}", response.text());
    } else if response.is_server_error() {
        println!("💥 Server error: {}", response.text());
    }

    println!("\n📊 Method 4: PUT request with manual JSON");

    // 🎯 PUT request with manual JSON construction
    let proxy_config = serde_json::json!({
        "name": "DIRECT",
        "type": "direct",
        "udp": true
    });

    let response = client
        .put("/proxies/DIRECT")
        .json(&proxy_config)
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
        200..=299 => println!("✅ Success: {}", response.text()),
        400..=499 => println!("❌ Client error {}: {}", response.status(), response.text()),
        500..=599 => println!("💥 Server error {}: {}", response.status(), response.text()),
        _ => println!(
            "🤷 Unknown status {}: {}",
            response.status(),
            response.text()
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

    println!("\n📊 Method 7: Custom HTTP method");

    // 🎯 Custom HTTP method
    let response = client
        .request("OPTIONS", "/")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;

    println!("✅ OPTIONS response status: {}", response.status());

    println!("\n📊 Method 8: Response inspection");

    // 🎯 Response inspection
    let response = client.get("/version").send().await?;

    println!("✅ Headers: {}", response.headers());
    println!("✅ Status: {}", response.status());
    println!("✅ Content length: {}", response.content_length());
    println!("✅ Is success: {}", response.is_success());
    println!("✅ Is error: {}", response.is_error());

    if response.is_success() {
        let version: serde_json::Value = response.json()?;
        println!("🎉 Version info: {version}");
    }

    println!("\n📊 Method 9: Backward compatibility");

    // 🎯 Backward compatible methods are still available
    let response = client.get_simple("/proxies").await?;
    println!("✅ Backward compatible GET status: {}", response.status);

    println!("\n🎯 Benefits of the new HTTP client API:");
    println!("📌 HTTP-like methods: get(), post(), put(), delete(), patch(), head()");
    println!("📌 Method chaining: .json_body().timeout().send()");
    println!("📌 Type-safe JSON: json_result<T>(), json_body<T>()");
    println!("📌 Rich response handling: is_success(), is_error(), content_length()");
    println!("📌 Fluent API: .json().timeout().send()");
    println!("📌 Error categorization: client_error(), server_error()");
    println!("📌 Backward compatible: old methods still work");

    Ok(())
}
