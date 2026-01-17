pub mod config;
pub mod errors;
pub mod http_client;
pub mod metrics;
pub mod parser_cache;
pub mod pool;
pub mod response;
pub mod retry;

#[cfg(feature = "client")]
pub mod ipc_http_client;

#[cfg(feature = "client")]
pub mod ipc_stream_client;

#[cfg(feature = "client")]
pub mod stream_client;

#[cfg(feature = "server")]
pub mod ipc_http_server;

#[cfg(feature = "server")]
pub mod ipc_stream_server;

pub use config::*;
pub use errors::*;
pub use metrics::{
    global_metrics, init_metrics, BufferPoolStats, HealthChecker, HealthReport, HealthStatus, MetricsCollector,
    MetricsSnapshot, ParserCacheStats,
};
pub use response::*;

#[cfg(feature = "client")]
pub use ipc_http_client::*;

#[cfg(feature = "client")]
pub use ipc_stream_client::*;

#[cfg(feature = "client")]
pub use stream_client::*;

#[cfg(feature = "server")]
pub use ipc_http_server::{IpcHttpServer, RequestContext, Router, ServerConfig};

#[cfg(feature = "server")]
pub use ipc_stream_server::*;

#[cfg(test)]
mod test_utils {
    use crate::config::GlobalConfig;
    use tokio::sync::oneshot;

    pub fn test_config() -> GlobalConfig {
        let mut config = GlobalConfig::default();
        config.client.default_timeout_ms = 5000;
        config.client.max_retries = 2;
        config.client.retry_delay_ms = 100;
        config.client.connection_timeout_ms = 1000;
        config.client.pool.max_size = 5;
        config.client.pool.min_idle = 2; // 确保 min_idle <= max_size
        config
    }

    pub fn _setup_test_server(_socket_path: &str) -> (tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
        // TODO: Fix server API integration
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            // Placeholder for server implementation
        });
        (handle, shutdown_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::*;

    #[test]
    fn test_config_validation() {
        let mut invalid_config = test_config();
        invalid_config.client.default_timeout_ms = 0;

        assert!(invalid_config.validate().is_err());

        let valid_config = test_config();
        assert!(valid_config.validate().is_ok());
    }

    #[test]
    fn test_config_builder() {
        use crate::config::ConfigBuilder;
        use std::time::Duration;

        let config = ConfigBuilder::new()
            .client_timeout(Duration::from_millis(3000))
            .max_retries(3)
            .enable_logging("debug")
            .enable_feature("caching")
            .build()
            .unwrap();

        assert_eq!(config.client.default_timeout_ms, 3000);
        assert_eq!(config.client.max_retries, 3);
        assert_eq!(config.logging.level, "debug");
        assert!(config.features.caching);
    }

    #[test]
    fn test_error_categorization() {
        use crate::errors::KodeBridgeError;

        let connection_error = KodeBridgeError::Connection {
            message: "Connection failed".to_string(),
        };
        assert!(connection_error.is_retriable());
        assert!(!connection_error.is_client_error());
        assert!(!connection_error.is_server_error());

        let config_error = KodeBridgeError::Configuration {
            message: "Invalid configuration".to_string(),
        };
        assert!(!config_error.is_retriable());

        let client_error = KodeBridgeError::ClientError { status: 400 };
        assert!(client_error.is_client_error());
        assert!(!client_error.is_server_error());

        let server_error = KodeBridgeError::ServerError { status: 500 };
        assert!(server_error.is_server_error());
        assert!(!server_error.is_client_error());
    }

    #[test]
    fn test_timeout_error() {
        use crate::errors::KodeBridgeError;

        let timeout_error = KodeBridgeError::timeout(5000);
        assert!(timeout_error.is_retriable());

        assert!(matches!(timeout_error, KodeBridgeError::Timeout { duration_ms } if duration_ms == 5000));
    }

    #[test]
    fn test_error_construction_helpers() {
        use crate::errors::KodeBridgeError;

        let conn_err = KodeBridgeError::connection("Test connection error");
        assert!(matches!(conn_err, KodeBridgeError::Connection { message } if message == "Test connection error"));

        let proto_err = KodeBridgeError::protocol("Protocol violation");
        assert!(matches!(proto_err, KodeBridgeError::Protocol { message } if message == "Protocol violation"));

        let config_err = KodeBridgeError::configuration("Bad config");
        assert!(matches!(config_err, KodeBridgeError::Configuration { message } if message == "Bad config"));

        let custom_err = KodeBridgeError::custom("Custom error message");
        assert!(matches!(custom_err, KodeBridgeError::Custom { message } if message == "Custom error message"));
    }

    #[test]
    fn test_pool_config() {
        use crate::pool::PoolConfig;

        let pool_config = PoolConfig {
            max_size: 10,
            min_idle: 2,
            max_idle_time_ms: 300_000,
            connection_timeout_ms: 30_000,
            retry_delay_ms: 100,
            max_retries: 3,
            max_concurrent_requests: 8,
            max_requests_per_second: Some(10.0),
        };

        assert_eq!(pool_config.max_size, 10);
        assert_eq!(pool_config.min_idle, 2);
        assert_eq!(pool_config.max_idle_time_ms, 300_000);

        let default_config = PoolConfig::default();
        assert_eq!(default_config.max_size, 64); // 更新为新的默认值
        assert_eq!(default_config.min_idle, 8); // 更新为新的默认值
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_path_security() {
        use crate::ipc_http_server::Router;

        let router = Router::new();

        // Test safe paths
        assert!(router.is_safe_path("/api/users"));
        assert!(router.is_safe_path("/"));
        assert!(router.is_safe_path("/data/file.json"));

        // Test unsafe paths - directory traversal
        assert!(!router.is_safe_path("/../etc/passwd"));
        assert!(!router.is_safe_path("/api/../../../etc/passwd"));
        assert!(!router.is_safe_path("/data\\..\\windows"));

        // Test paths with invalid characters
        assert!(!router.is_safe_path("/api/users\0"));
        assert!(!router.is_safe_path("/api/\x01users"));

        // Test paths not starting with /
        assert!(!router.is_safe_path("api/users"));
        assert!(!router.is_safe_path("../etc/passwd"));

        // Test excessively long paths
        let long_path = "/".to_string() + &"a".repeat(3000);
        assert!(!router.is_safe_path(&long_path));
    }

    #[test]
    fn test_metrics_integration() {
        use crate::metrics::global_metrics;

        let metrics = global_metrics();

        // Test request tracking
        {
            let tracker = metrics.request_start("GET");
            std::thread::sleep(std::time::Duration::from_millis(1));
            tracker.success(200);
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.successful_requests, 1);
        assert_eq!(snapshot.active_requests, 0);

        // Test connection tracking
        metrics.connection_created(true);
        metrics.connection_created(false);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_connections, 2);
        assert_eq!(snapshot.pool_hits, 1);
        assert_eq!(snapshot.pool_misses, 1);
    }

    #[test]
    fn test_health_checker() {
        use crate::metrics::{HealthChecker, HealthStatus, MetricsCollector};
        use std::sync::Arc;

        let metrics = Arc::new(MetricsCollector::new());
        let health_checker = HealthChecker::new(metrics);

        let report = health_checker.check_health();
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.issues.is_empty());
    }
}
