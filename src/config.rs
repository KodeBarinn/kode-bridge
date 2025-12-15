use crate::errors::{KodeBridgeError, Result};
use crate::pool::PoolConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Global configuration for kode-bridge
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    /// Default client configuration
    pub client: ClientGlobalConfig,
    /// Streaming client configuration
    pub streaming: StreamingGlobalConfig,
    /// Logging configuration
    pub logging: LoggingConfig,
    /// Feature flags
    pub features: FeatureFlags,
}

/// Client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientGlobalConfig {
    /// Default timeout for requests
    pub default_timeout_ms: u64,
    /// Enable connection pooling by default
    pub enable_pooling: bool,
    /// Pool configuration
    pub pool: PoolConfig,
    /// Retry configuration
    pub max_retries: usize,
    pub retry_delay_ms: u64,
    /// Connection timeout
    pub connection_timeout_ms: u64,
}

impl Default for ClientGlobalConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 10_000, // Reduce default timeout to 10 seconds
            enable_pooling: true,
            pool: PoolConfig::default(),
            max_retries: 5,               // Increase retry attempts
            retry_delay_ms: 50,           // Reduce retry delay
            connection_timeout_ms: 5_000, // Reduce connection timeout
        }
    }
}

/// Streaming client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingGlobalConfig {
    /// Default timeout for streaming connections
    pub default_timeout_ms: u64,
    /// Buffer size for streaming
    pub buffer_size: usize,
    /// Max retries for streaming connections
    pub max_retries: usize,
    /// Retry delay for streaming connections
    pub retry_delay_ms: u64,
}

impl Default for StreamingGlobalConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 60_000,
            buffer_size: 8192,
            max_retries: 3,
            retry_delay_ms: 100,
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Enable tracing
    pub enabled: bool,
    /// Log level (trace, debug, info, warn, error)
    pub level: String,
    /// Enable structured logging
    pub structured: bool,
    /// Log request/response details
    pub log_requests: bool,
    /// Log connection events
    pub log_connections: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: "info".to_string(),
            structured: false,
            log_requests: false,
            log_connections: false,
        }
    }
}

/// Feature flags for experimental or optional features
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    /// Enable HTTP/2 support (future feature)
    pub http2_support: bool,
    /// Enable compression
    pub compression: bool,
    /// Enable request/response caching
    pub caching: bool,
    /// Enable metrics collection
    pub metrics: bool,
    /// Enable connection keep-alive optimization
    pub keep_alive: bool,
    /// Enable automatic reconnection
    pub auto_reconnect: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            http2_support: false,
            compression: false,
            caching: false,
            metrics: false,
            keep_alive: true,
            auto_reconnect: true,
        }
    }
}

impl GlobalConfig {
    /// Load configuration from a TOML file
    pub fn from_toml_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to read config file: {}", e))
        })?;

        toml::from_str(&content).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to parse TOML config: {}", e))
        })
    }

    /// Load configuration from a JSON file
    pub fn from_json_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to read config file: {}", e))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to parse JSON config: {}", e))
        })
    }

    /// Save configuration to a TOML file
    pub fn save_toml_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to serialize config: {}", e))
        })?;

        std::fs::write(path, content).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to write config file: {}", e))
        })
    }

    /// Save configuration to a JSON file
    pub fn save_json_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to serialize config: {}", e))
        })?;

        std::fs::write(path, content).map_err(|e| {
            KodeBridgeError::configuration(format!("Failed to write config file: {}", e))
        })
    }

    /// Apply environment variable overrides
    pub fn apply_env_overrides(&mut self) {
        if let Ok(timeout) = std::env::var("KODE_BRIDGE_TIMEOUT_MS") {
            if let Ok(timeout_ms) = timeout.parse::<u64>() {
                self.client.default_timeout_ms = timeout_ms;
            }
        }

        if let Ok(pooling) = std::env::var("KODE_BRIDGE_ENABLE_POOLING") {
            self.client.enable_pooling = pooling.to_lowercase() == "true";
        }

        if let Ok(retries) = std::env::var("KODE_BRIDGE_MAX_RETRIES") {
            if let Ok(retries_num) = retries.parse::<usize>() {
                self.client.max_retries = retries_num;
            }
        }

        if let Ok(log_level) = std::env::var("KODE_BRIDGE_LOG_LEVEL") {
            self.logging.level = log_level;
            self.logging.enabled = true;
        }

        if let Ok(pool_size) = std::env::var("KODE_BRIDGE_POOL_SIZE") {
            if let Ok(size) = pool_size.parse::<usize>() {
                self.client.pool.max_size = size;
            }
        }
    }

    /// Convert client timeout to Duration
    pub const fn client_timeout(&self) -> Duration {
        Duration::from_millis(self.client.default_timeout_ms)
    }

    /// Convert streaming timeout to Duration
    pub const fn streaming_timeout(&self) -> Duration {
        Duration::from_millis(self.streaming.default_timeout_ms)
    }

    /// Convert retry delay to Duration
    pub const fn retry_delay(&self) -> Duration {
        Duration::from_millis(self.client.retry_delay_ms)
    }

    /// Convert connection timeout to Duration
    pub const fn connection_timeout(&self) -> Duration {
        Duration::from_millis(self.client.connection_timeout_ms)
    }

    /// Check if feature is enabled
    pub fn is_feature_enabled(&self, feature: &str) -> bool {
        match feature {
            "http2" => self.features.http2_support,
            "compression" => self.features.compression,
            "caching" => self.features.caching,
            "metrics" => self.features.metrics,
            "keep_alive" => self.features.keep_alive,
            "auto_reconnect" => self.features.auto_reconnect,
            _ => false,
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.client.default_timeout_ms == 0 {
            return Err(KodeBridgeError::configuration(
                "Client timeout cannot be zero",
            ));
        }

        if self.client.max_retries > 10 {
            return Err(KodeBridgeError::configuration(
                "Max retries should be <= 10",
            ));
        }

        if self.client.pool.max_size == 0 {
            return Err(KodeBridgeError::configuration(
                "Pool max size cannot be zero",
            ));
        }

        if self.client.pool.min_idle > self.client.pool.max_size {
            return Err(KodeBridgeError::configuration(
                "Pool min_idle cannot be greater than max_size",
            ));
        }

        if !["trace", "debug", "info", "warn", "error"].contains(&self.logging.level.as_str()) {
            return Err(KodeBridgeError::configuration("Invalid log level"));
        }

        Ok(())
    }
}

/// Configuration builder for fluent configuration setup
pub struct ConfigBuilder {
    config: GlobalConfig,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: GlobalConfig::default(),
        }
    }

    /// Set client timeout
    pub const fn client_timeout(mut self, timeout: Duration) -> Self {
        self.config.client.default_timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Enable or disable connection pooling
    pub const fn enable_pooling(mut self, enabled: bool) -> Self {
        self.config.client.enable_pooling = enabled;
        self
    }

    /// Set pool configuration
    pub const fn pool_config(mut self, pool_config: PoolConfig) -> Self {
        self.config.client.pool = pool_config;
        self
    }

    /// Set max retries
    pub const fn max_retries(mut self, retries: usize) -> Self {
        self.config.client.max_retries = retries;
        self
    }

    /// Enable logging
    pub fn enable_logging(mut self, level: &str) -> Self {
        self.config.logging.enabled = true;
        self.config.logging.level = level.to_string();
        self
    }

    /// Enable feature
    pub fn enable_feature(mut self, feature: &str) -> Self {
        match feature {
            "compression" => self.config.features.compression = true,
            "caching" => self.config.features.caching = true,
            "metrics" => self.config.features.metrics = true,
            "keep_alive" => self.config.features.keep_alive = true,
            "auto_reconnect" => self.config.features.auto_reconnect = true,
            _ => {}
        }
        self
    }

    /// Build the configuration
    pub fn build(self) -> Result<GlobalConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
