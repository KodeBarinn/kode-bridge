use kode_bridge::{
    global_metrics, init_metrics, ConfigBuilder, HealthChecker, HealthStatus, MetricsSnapshot, ParserCacheStats,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better visibility
    tracing_subscriber::fmt::init();

    println!("🚀 Kode-Bridge Performance Monitoring Demo");
    println!("==========================================");

    // Initialize metrics system
    let metrics = init_metrics();
    println!("✅ Metrics system initialized");

    // Create health checker
    let health_checker = HealthChecker::new(Arc::clone(&metrics));
    println!("✅ Health checker created");

    // Create a test configuration
    let _config = ConfigBuilder::new()
        .client_timeout(Duration::from_millis(1000))
        .max_retries(3)
        .enable_logging("info")
        .enable_feature("metrics")
        .build()?;

    println!("✅ Configuration created");

    // Demo: Simulated load testing
    println!("\n📊 Running simulated load test...");

    // Simulate some requests (would normally connect to actual server)
    for i in 1..=10 {
        let tracker = global_metrics().request_start("GET");

        // Simulate processing time
        sleep(Duration::from_millis(50 + (i * 10))).await;

        // Simulate success/failure
        if i % 4 == 0 {
            tracker.failure("SimulatedError");
        } else {
            tracker.success(200);
        }

        println!("  📈 Request {} completed", i);
    }

    // Simulate connection events
    for _ in 0..5 {
        global_metrics().connection_created(true); // From pool
    }
    for _ in 0..2 {
        global_metrics().connection_created(false); // Direct
    }
    global_metrics().connection_failed();

    println!("✅ Load test completed");

    // Get metrics snapshot
    println!("\n📈 Current Metrics:");
    println!("==================");

    let snapshot: MetricsSnapshot = global_metrics().snapshot();
    print_metrics_summary(&snapshot);

    // Check health
    println!("\n🏥 Health Check:");
    println!("================");

    let health_report = health_checker.check_health();
    print_health_report(&health_report);

    // Print detailed metrics using built-in summary
    println!("\n📊 Detailed Metrics Summary:");
    println!("============================");
    global_metrics().print_summary();

    // Demonstrate parser cache
    println!("\n🧠 Parser Cache Statistics:");
    println!("===========================");
    demonstrate_parser_cache();

    println!("\n✨ Demo completed successfully!");
    println!("📝 In a real application, you would:");
    println!("   • Set up periodic health checks");
    println!("   • Export metrics to monitoring systems");
    println!("   • Set up alerts based on thresholds");
    println!("   • Use metrics for auto-scaling decisions");

    Ok(())
}

fn print_metrics_summary(snapshot: &MetricsSnapshot) {
    println!("┌─────────────────────────────────────────┐");
    println!("│               REQUESTS                  │");
    println!("├─────────────────────────────────────────┤");
    println!("│ Total:      {:>10}               │", snapshot.total_requests);
    println!("│ Successful: {:>10}               │", snapshot.successful_requests);
    println!("│ Failed:     {:>10}               │", snapshot.failed_requests);
    println!("│ Active:     {:>10}               │", snapshot.active_requests);
    println!("└─────────────────────────────────────────┘");

    println!();
    println!("┌─────────────────────────────────────────┐");
    println!("│             CONNECTIONS                 │");
    println!("├─────────────────────────────────────────┤");
    println!("│ Total:      {:>10}               │", snapshot.total_connections);
    println!("│ Active:     {:>10}               │", snapshot.active_connections);
    println!("│ Pool Hits:  {:>10}               │", snapshot.pool_hits);
    println!("│ Pool Miss:  {:>10}               │", snapshot.pool_misses);
    println!("└─────────────────────────────────────────┘");

    println!();
    println!("┌─────────────────────────────────────────┐");
    println!("│             PERFORMANCE                 │");
    println!("├─────────────────────────────────────────┤");
    if let Some(avg) = snapshot.avg_latency {
        println!("│ Avg Latency: {:>8} ms           │", avg.as_millis());
    }
    if let Some(p95) = snapshot.p95_latency {
        println!("│ P95 Latency: {:>8} ms           │", p95.as_millis());
    }
    if let Some(p99) = snapshot.p99_latency {
        println!("│ P99 Latency: {:>8} ms           │", p99.as_millis());
    }
    println!("│ Throughput:  {:>8.2} req/s       │", snapshot.requests_per_second);
    println!("└─────────────────────────────────────────┘");

    if snapshot.total_errors > 0 {
        println!();
        println!("┌─────────────────────────────────────────┐");
        println!("│               ERRORS                    │");
        println!("├─────────────────────────────────────────┤");
        println!("│ Total:      {:>10}               │", snapshot.total_errors);
        println!("│ Timeouts:   {:>10}               │", snapshot.timeout_errors);
        println!("│ Connection: {:>10}               │", snapshot.connection_errors);
        println!("└─────────────────────────────────────────┘");
    }
}

fn print_health_report(report: &kode_bridge::HealthReport) {
    let status_icon = match report.status {
        HealthStatus::Healthy => "✅",
        HealthStatus::Warning => "⚠️",
        HealthStatus::Critical => "❌",
    };

    println!("{} Status: {:?}", status_icon, report.status);

    if !report.issues.is_empty() {
        println!("📋 Issues detected:");
        for issue in &report.issues {
            println!("   • {}", issue);
        }
    } else {
        println!("🎉 No issues detected - system is healthy!");
    }
}

fn demonstrate_parser_cache() {
    use kode_bridge::parser_cache::global_parser_cache;

    let cache = global_parser_cache();

    // Simulate some parsing operations
    for _ in 0..5 {
        let mut parser = cache.get();
        let http_response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let _ = parser.parse_response(http_response);
    }

    // Update metrics
    global_metrics().update_parser_cache_stats(ParserCacheStats {
        cache_size: cache.size(),
        cache_hits: 15,
        cache_misses: 5,
        hit_rate: 0.75,
    });

    println!("Cache size: {} parsers", cache.size());
    println!("Hit rate: 75%");
}
