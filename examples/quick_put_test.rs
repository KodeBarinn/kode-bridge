use kode_bridge::IpcHttpClient;
use serde_json::json;
use std::time::Instant;

/// Quick PUT performance verification tool
/// Usage: Start the server first, then run this test
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Quick PUT Performance Test");

    let socket_path = "/tmp/test.sock";

    let client = match IpcHttpClient::new(socket_path) {
        Ok(client) => {
            println!("✅ Connected to server at {}", socket_path);
            client
        }
        Err(e) => {
            println!("❌ Cannot connect to server: {}", e);
            println!("💡 Start server with: cargo run --example http_server --features server");
            return Ok(());
        }
    };

    // Preheat the connection pool for PUT requests
    println!("🔥 Preheating connections for PUT requests...");
    client.preheat_for_puts(3).await;

    // Test data
    let small_data = json!({"test": "small", "data": "x".repeat(1000)}); // ~1KB
    let medium_data = json!({"test": "medium", "data": "x".repeat(50000)}); // ~50KB
    let large_data = json!({"test": "large", "data": "x".repeat(500000)}); // ~500KB

    println!("\n📊 Testing different PUT request sizes:");

    // Small data PUT test
    let start = Instant::now();
    for i in 0..10 {
        let response = client
            .put(&format!("/test/small/{}", i))
            .json_body(&small_data)
            .optimize_for_put()
            .expected_size(1000)
            .send()
            .await?;

        if i == 0 {
            println!("   Small PUT (1KB): {} - {:?}", response.status(), start.elapsed());
        }
    }
    let small_total = start.elapsed();
    println!(
        "   ✅ 10 small PUTs completed in {:?} (avg: {:?})",
        small_total,
        small_total / 10
    );

    // Medium data PUT test
    let start = Instant::now();
    for i in 0..5 {
        let response = client
            .put(&format!("/test/medium/{}", i))
            .json_body(&medium_data)
            .optimize_for_put()
            .expected_size(50000)
            .send()
            .await?;

        if i == 0 {
            println!("   Medium PUT (50KB): {} - {:?}", response.status(), start.elapsed());
        }
    }
    let medium_total = start.elapsed();
    println!(
        "   ✅ 5 medium PUTs completed in {:?} (avg: {:?})",
        medium_total,
        medium_total / 5
    );

    // Large data PUT test
    let start = Instant::now();
    for i in 0..3 {
        let response = client
            .put(&format!("/test/large/{}", i))
            .json_body(&large_data)
            .optimize_for_put()
            .expected_size(500000)
            .send()
            .await?;

        if i == 0 {
            println!("   Large PUT (500KB): {} - {:?}", response.status(), start.elapsed());
        }
    }
    let large_total = start.elapsed();
    println!(
        "   ✅ 3 large PUTs completed in {:?} (avg: {:?})",
        large_total,
        large_total / 3
    );

    // Concurrent PUT test
    println!("\n⚡ Testing concurrent PUTs:");
    let start = Instant::now();

    let mut futures = Vec::new();
    for i in 0..8 {
        let future = client
            .put(&format!("/test/concurrent/{}", i))
            .json_body(&medium_data)
            .optimize_for_put()
            .expected_size(50000)
            .send();
        futures.push(future);
    }

    let results = futures::future::join_all(futures).await;
    let concurrent_duration = start.elapsed();

    let successful = results.iter().filter(|r| r.is_ok()).count();
    println!(
        "   ✅ {}/8 concurrent PUTs completed in {:?} (avg: {:?})",
        successful,
        concurrent_duration,
        concurrent_duration / 8
    );

    // Batch PUT test
    println!("\n📦 Testing batch PUTs:");
    let start = Instant::now();

    let batch_requests: Vec<_> = (0..5)
        .map(|i| (format!("/test/batch/{}", i), medium_data.clone()))
        .collect();

    let batch_responses = client.put_batch(batch_requests).await?;
    let batch_duration = start.elapsed();

    println!(
        "   ✅ {} batch PUTs completed in {:?} (avg: {:?})",
        batch_responses.len(),
        batch_duration,
        batch_duration / batch_responses.len() as u32
    );

    println!("\n🎉 Performance test completed!");
    println!("💡 Key optimizations active:");
    println!("   - Smart timeout calculation based on data size");
    println!("   - Fresh connection pool for PUT requests");
    println!("   - Optimized HTTP parsing and serialization");
    println!("   - Zero-copy buffer management");
    println!("   - Reduced memory allocations");

    Ok(())
}
