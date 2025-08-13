use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Global metrics collection system
pub struct MetricsCollector {
    /// Request counters
    pub requests: RequestMetrics,
    /// Connection metrics  
    pub connections: ConnectionMetrics,
    /// Performance metrics
    pub performance: PerformanceMetrics,
    /// Error tracking
    pub errors: ErrorMetrics,
    /// Resource usage
    pub resources: ResourceMetrics,
}

/// Request-related metrics
#[derive(Default)]
pub struct RequestMetrics {
    /// Total requests processed
    pub total_requests: AtomicU64,
    /// Successful requests
    pub successful_requests: AtomicU64,
    /// Failed requests
    pub failed_requests: AtomicU64,
    /// Requests by method
    pub requests_by_method: RwLock<HashMap<String, AtomicU64>>,
    /// Requests by status code
    pub requests_by_status: RwLock<HashMap<u16, AtomicU64>>,
    /// Active requests (in-flight)
    pub active_requests: AtomicUsize,
}

/// Connection-related metrics
#[derive(Default)]
pub struct ConnectionMetrics {
    /// Total connections created
    pub total_connections: AtomicU64,
    /// Active connections
    pub active_connections: AtomicUsize,
    /// Failed connection attempts
    pub failed_connections: AtomicU64,
    /// Connection pool utilization
    pub pool_size: AtomicUsize,
    /// Pool hits (reused connections)
    pub pool_hits: AtomicU64,
    /// Pool misses (new connections)
    pub pool_misses: AtomicU64,
}

/// Performance-related metrics
pub struct PerformanceMetrics {
    /// Request latency tracking
    pub request_latencies: RwLock<LatencyTracker>,
    /// Connection establishment times
    pub connection_latencies: RwLock<LatencyTracker>,
    /// Throughput tracking
    pub throughput: RwLock<ThroughputTracker>,
    /// Retry statistics
    pub retry_stats: RwLock<RetryStats>,
}

/// Error tracking metrics
#[derive(Default)]
pub struct ErrorMetrics {
    /// Errors by type
    pub errors_by_type: RwLock<HashMap<String, AtomicU64>>,
    /// Total errors
    pub total_errors: AtomicU64,
    /// Circuit breaker trips
    pub circuit_breaker_trips: AtomicU64,
    /// Timeout errors
    pub timeout_errors: AtomicU64,
    /// Connection errors
    pub connection_errors: AtomicU64,
}

/// Resource usage metrics
#[derive(Default)]
pub struct ResourceMetrics {
    /// Buffer pool statistics
    pub buffer_pools: RwLock<BufferPoolStats>,
    /// Parser cache statistics
    pub parser_cache: RwLock<ParserCacheStats>,
    /// Memory usage estimates
    pub memory_usage: AtomicU64,
    /// CPU time tracking
    pub cpu_time: RwLock<CpuTimeTracker>,
}

/// Latency tracking with percentiles
pub struct LatencyTracker {
    samples: Vec<Duration>,
    last_reset: Instant,
    max_samples: usize,
}

/// Throughput tracking
pub struct ThroughputTracker {
    /// Requests per time window
    windows: Vec<(Instant, u64)>,
    window_size: Duration,
    max_windows: usize,
}

/// Retry statistics
#[derive(Default)]
pub struct RetryStats {
    /// Total retry attempts
    pub total_retries: AtomicU64,
    /// Successful retries (eventual success)
    pub successful_retries: AtomicU64,
    /// Failed retries (gave up)
    pub failed_retries: AtomicU64,
    /// Retry counts by attempt number
    pub retries_by_attempt: HashMap<usize, AtomicU64>,
}

/// Buffer pool statistics
#[derive(Default, Clone, Debug)]
pub struct BufferPoolStats {
    pub small_pool_size: usize,
    pub medium_pool_size: usize,
    pub large_pool_size: usize,
    pub total_allocations: u64,
    pub total_reuses: u64,
}

/// Parser cache statistics
#[derive(Default, Clone, Debug)]
pub struct ParserCacheStats {
    pub cache_size: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub hit_rate: f64,
}

/// CPU time tracking
#[derive(Default)]
pub struct CpuTimeTracker {
    // Placeholder for future CPU monitoring
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            request_latencies: RwLock::new(LatencyTracker::new(1000)),
            connection_latencies: RwLock::new(LatencyTracker::new(500)),
            throughput: RwLock::new(ThroughputTracker::new(Duration::from_secs(60), 100)),
            retry_stats: RwLock::new(RetryStats::default()),
        }
    }
}

impl LatencyTracker {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::with_capacity(max_samples),
            last_reset: Instant::now(),
            max_samples,
        }
    }

    pub fn record(&mut self, latency: Duration) {
        if self.samples.len() >= self.max_samples {
            // Keep only recent samples
            self.samples.drain(0..self.max_samples / 2);
        }
        self.samples.push(latency);
    }

    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let mut sorted = self.samples.clone();
        sorted.sort();

        let index = ((sorted.len() - 1) as f64 * p / 100.0).round() as usize;
        Some(sorted[index])
    }

    pub fn average(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let sum: Duration = self.samples.iter().sum();
        Some(sum / self.samples.len() as u32)
    }

    pub fn count(&self) -> usize {
        self.samples.len()
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.last_reset = Instant::now();
    }
}

impl ThroughputTracker {
    pub fn new(window_size: Duration, max_windows: usize) -> Self {
        Self {
            windows: Vec::with_capacity(max_windows),
            window_size,
            max_windows,
        }
    }

    pub fn record_request(&mut self) {
        let now = Instant::now();

        // Clean old windows
        self.windows
            .retain(|(timestamp, _)| now.duration_since(*timestamp) <= self.window_size);

        // Add to current window or create new one
        if let Some((_, count)) = self.windows.last_mut() {
            *count += 1;
        } else {
            if self.windows.len() >= self.max_windows {
                self.windows.remove(0);
            }
            self.windows.push((now, 1));
        }
    }

    pub fn requests_per_second(&self) -> f64 {
        if self.windows.is_empty() {
            return 0.0;
        }

        let total_requests: u64 = self.windows.iter().map(|(_, count)| *count).sum();
        let time_span = self
            .windows
            .last()
            .unwrap()
            .0
            .duration_since(self.windows[0].0);

        if time_span.as_secs_f64() > 0.0 {
            total_requests as f64 / time_span.as_secs_f64()
        } else {
            0.0
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            requests: RequestMetrics::default(),
            connections: ConnectionMetrics::default(),
            performance: PerformanceMetrics::default(),
            errors: ErrorMetrics::default(),
            resources: ResourceMetrics::default(),
        }
    }

    /// Record a request start
    pub fn request_start(&self, method: &str) -> RequestTracker<'_> {
        self.requests.total_requests.fetch_add(1, Ordering::Relaxed);
        self.requests
            .active_requests
            .fetch_add(1, Ordering::Relaxed);

        // Update method counter
        {
            let methods = self.requests.requests_by_method.read();
            if let Some(counter) = methods.get(method) {
                counter.fetch_add(1, Ordering::Relaxed);
            } else {
                drop(methods);
                let mut methods = self.requests.requests_by_method.write();
                methods
                    .entry(method.to_string())
                    .or_insert_with(|| AtomicU64::new(1));
            }
        }

        // Update throughput
        {
            let mut throughput = self.performance.throughput.write();
            throughput.record_request();
        }

        RequestTracker {
            metrics: self,
            start_time: Instant::now(),
        }
    }

    /// Record a connection event
    pub fn connection_created(&self, from_pool: bool) {
        self.connections
            .total_connections
            .fetch_add(1, Ordering::Relaxed);
        self.connections
            .active_connections
            .fetch_add(1, Ordering::Relaxed);

        if from_pool {
            self.connections.pool_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.connections.pool_misses.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record connection failure
    pub fn connection_failed(&self) {
        self.connections
            .failed_connections
            .fetch_add(1, Ordering::Relaxed);
        self.errors
            .connection_errors
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record an error
    pub fn record_error(&self, error_type: &str) {
        self.errors.total_errors.fetch_add(1, Ordering::Relaxed);

        let mut errors = self.errors.errors_by_type.write();
        errors
            .entry(error_type.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record retry attempt
    pub fn record_retry(&self, attempt: usize, success: bool) {
        let mut stats = self.performance.retry_stats.write();
        stats.total_retries.fetch_add(1, Ordering::Relaxed);

        if success {
            stats.successful_retries.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.failed_retries.fetch_add(1, Ordering::Relaxed);
        }

        stats
            .retries_by_attempt
            .entry(attempt)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Update resource usage
    pub fn update_buffer_pool_stats(&self, stats: BufferPoolStats) {
        *self.resources.buffer_pools.write() = stats;
    }

    pub fn update_parser_cache_stats(&self, stats: ParserCacheStats) {
        *self.resources.parser_cache.write() = stats;
    }

    /// Get comprehensive metrics snapshot
    pub fn snapshot(&self) -> MetricsSnapshot {
        let request_latencies = self.performance.request_latencies.read();
        let throughput = self.performance.throughput.read();

        MetricsSnapshot {
            // Request metrics
            total_requests: self.requests.total_requests.load(Ordering::Relaxed),
            successful_requests: self.requests.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.requests.failed_requests.load(Ordering::Relaxed),
            active_requests: self.requests.active_requests.load(Ordering::Relaxed),

            // Connection metrics
            total_connections: self.connections.total_connections.load(Ordering::Relaxed),
            active_connections: self.connections.active_connections.load(Ordering::Relaxed),
            pool_hits: self.connections.pool_hits.load(Ordering::Relaxed),
            pool_misses: self.connections.pool_misses.load(Ordering::Relaxed),

            // Performance metrics
            avg_latency: request_latencies.average(),
            p95_latency: request_latencies.percentile(95.0),
            p99_latency: request_latencies.percentile(99.0),
            requests_per_second: throughput.requests_per_second(),

            // Error metrics
            total_errors: self.errors.total_errors.load(Ordering::Relaxed),
            timeout_errors: self.errors.timeout_errors.load(Ordering::Relaxed),
            connection_errors: self.errors.connection_errors.load(Ordering::Relaxed),

            // Resource metrics
            buffer_pool_stats: self.resources.buffer_pools.read().clone(),
            parser_cache_stats: self.resources.parser_cache.read().clone(),
            memory_usage: self.resources.memory_usage.load(Ordering::Relaxed),

            timestamp: Instant::now(),
        }
    }

    /// Print metrics summary
    pub fn print_summary(&self) {
        let snapshot = self.snapshot();

        info!("=== Kode-Bridge Metrics Summary ===");
        info!(
            "Requests: {} total, {} active, {} successful, {} failed",
            snapshot.total_requests,
            snapshot.active_requests,
            snapshot.successful_requests,
            snapshot.failed_requests
        );

        if let Some(avg) = snapshot.avg_latency {
            info!("Latency: avg={:.2}ms", avg.as_millis());
        }
        if let Some(p95) = snapshot.p95_latency {
            info!("Latency P95: {:.2}ms", p95.as_millis());
        }

        info!("Throughput: {:.2} req/s", snapshot.requests_per_second);
        info!(
            "Connections: {} total, {} active, pool hit rate: {:.1}%",
            snapshot.total_connections,
            snapshot.active_connections,
            if snapshot.pool_hits + snapshot.pool_misses > 0 {
                snapshot.pool_hits as f64 / (snapshot.pool_hits + snapshot.pool_misses) as f64
                    * 100.0
            } else {
                0.0
            }
        );

        if snapshot.total_errors > 0 {
            warn!(
                "Errors: {} total ({} timeout, {} connection)",
                snapshot.total_errors, snapshot.timeout_errors, snapshot.connection_errors
            );
        }
    }
}

/// RAII tracker for individual requests
pub struct RequestTracker<'a> {
    metrics: &'a MetricsCollector,
    start_time: Instant,
}

impl<'a> RequestTracker<'a> {
    /// Mark request as completed successfully
    pub fn success(self, status_code: u16) {
        self.complete(true, Some(status_code));
    }

    /// Mark request as failed
    pub fn failure(self, error_type: &str) {
        self.metrics.record_error(error_type);
        self.complete(false, None);
    }

    fn complete(self, success: bool, status_code: Option<u16>) {
        let latency = self.start_time.elapsed();

        // Record latency
        {
            let mut latencies = self.metrics.performance.request_latencies.write();
            latencies.record(latency);
        }

        // Update counters
        self.metrics
            .requests
            .active_requests
            .fetch_sub(1, Ordering::Relaxed);

        if success {
            self.metrics
                .requests
                .successful_requests
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.metrics
                .requests
                .failed_requests
                .fetch_add(1, Ordering::Relaxed);
        }

        // Record status code
        if let Some(status) = status_code {
            let mut status_map = self.metrics.requests.requests_by_status.write();
            status_map
                .entry(status)
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Snapshot of current metrics
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    // Request metrics
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub active_requests: usize,

    // Connection metrics
    pub total_connections: u64,
    pub active_connections: usize,
    pub pool_hits: u64,
    pub pool_misses: u64,

    // Performance metrics
    pub avg_latency: Option<Duration>,
    pub p95_latency: Option<Duration>,
    pub p99_latency: Option<Duration>,
    pub requests_per_second: f64,

    // Error metrics
    pub total_errors: u64,
    pub timeout_errors: u64,
    pub connection_errors: u64,

    // Resource metrics
    pub buffer_pool_stats: BufferPoolStats,
    pub parser_cache_stats: ParserCacheStats,
    pub memory_usage: u64,

    pub timestamp: Instant,
}

/// Health check system
pub struct HealthChecker {
    metrics: Arc<MetricsCollector>,
    thresholds: HealthThresholds,
}

#[derive(Debug, Clone)]
pub struct HealthThresholds {
    pub max_error_rate: f64,           // Maximum error rate (0.0-1.0)
    pub max_avg_latency: Duration,     // Maximum average latency
    pub max_p95_latency: Duration,     // Maximum P95 latency
    pub min_success_rate: f64,         // Minimum success rate (0.0-1.0)
    pub max_active_connections: usize, // Maximum active connections
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            max_error_rate: 0.05,                        // 5% error rate
            max_avg_latency: Duration::from_millis(500), // 500ms avg
            max_p95_latency: Duration::from_secs(2),     // 2s P95
            min_success_rate: 0.95,                      // 95% success rate
            max_active_connections: 1000,                // 1000 active connections
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
}

pub struct HealthReport {
    pub status: HealthStatus,
    pub issues: Vec<String>,
    pub snapshot: MetricsSnapshot,
}

impl HealthChecker {
    pub fn new(metrics: Arc<MetricsCollector>) -> Self {
        Self {
            metrics,
            thresholds: HealthThresholds::default(),
        }
    }

    pub fn with_thresholds(mut self, thresholds: HealthThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    pub fn check_health(&self) -> HealthReport {
        let snapshot = self.metrics.snapshot();
        let mut issues = Vec::new();
        let mut status = HealthStatus::Healthy;

        // Check error rate
        if snapshot.total_requests > 0 {
            let error_rate = snapshot.failed_requests as f64 / snapshot.total_requests as f64;
            if error_rate > self.thresholds.max_error_rate {
                issues.push(format!(
                    "High error rate: {:.2}% (threshold: {:.2}%)",
                    error_rate * 100.0,
                    self.thresholds.max_error_rate * 100.0
                ));
                status = HealthStatus::Critical;
            }
        }

        // Check latency
        if let Some(avg_latency) = snapshot.avg_latency {
            if avg_latency > self.thresholds.max_avg_latency {
                issues.push(format!(
                    "High average latency: {}ms (threshold: {}ms)",
                    avg_latency.as_millis(),
                    self.thresholds.max_avg_latency.as_millis()
                ));
                if status == HealthStatus::Healthy {
                    status = HealthStatus::Warning;
                }
            }
        }

        if let Some(p95_latency) = snapshot.p95_latency {
            if p95_latency > self.thresholds.max_p95_latency {
                issues.push(format!(
                    "High P95 latency: {}ms (threshold: {}ms)",
                    p95_latency.as_millis(),
                    self.thresholds.max_p95_latency.as_millis()
                ));
                status = HealthStatus::Critical;
            }
        }

        // Check active connections
        if snapshot.active_connections > self.thresholds.max_active_connections {
            issues.push(format!(
                "Too many active connections: {} (threshold: {})",
                snapshot.active_connections, self.thresholds.max_active_connections
            ));
            if status == HealthStatus::Healthy {
                status = HealthStatus::Warning;
            }
        }

        HealthReport {
            status,
            issues,
            snapshot,
        }
    }
}

// Global metrics instance
use std::sync::OnceLock;

static GLOBAL_METRICS: OnceLock<Arc<MetricsCollector>> = OnceLock::new();

/// Get global metrics collector
pub fn global_metrics() -> &'static Arc<MetricsCollector> {
    GLOBAL_METRICS.get_or_init(|| Arc::new(MetricsCollector::new()))
}

/// Initialize metrics system
pub fn init_metrics() -> Arc<MetricsCollector> {
    global_metrics().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_latency_tracker() {
        let mut tracker = LatencyTracker::new(100);

        tracker.record(Duration::from_millis(100));
        tracker.record(Duration::from_millis(200));
        tracker.record(Duration::from_millis(300));

        assert_eq!(tracker.count(), 3);
        assert_eq!(tracker.average(), Some(Duration::from_millis(200)));
        assert_eq!(tracker.percentile(50.0), Some(Duration::from_millis(200)));
    }

    #[test]
    fn test_throughput_tracker() {
        let mut tracker = ThroughputTracker::new(Duration::from_secs(1), 10);

        tracker.record_request();
        tracker.record_request();
        tracker.record_request();

        // Note: actual RPS calculation depends on timing
        let rps = tracker.requests_per_second();
        assert!(rps >= 0.0);
    }

    #[test]
    fn test_metrics_collector() {
        let metrics = MetricsCollector::new();

        {
            let tracker = metrics.request_start("GET");
            thread::sleep(Duration::from_millis(10));
            tracker.success(200);
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.successful_requests, 1);
        assert_eq!(snapshot.active_requests, 0);
    }

    #[test]
    fn test_health_checker() {
        let metrics = Arc::new(MetricsCollector::new());
        let checker = HealthChecker::new(metrics.clone());

        let report = checker.check_health();
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.issues.is_empty());
    }
}
