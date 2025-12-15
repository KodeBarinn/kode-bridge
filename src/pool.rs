use crate::errors::{KodeBridgeError, Result};
use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::Name;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{debug, trace, warn};

/// Configuration for connection pool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool
    pub max_size: usize,
    /// Minimum number of idle connections to maintain
    pub min_idle: usize,
    /// Maximum time a connection can be idle before being closed (in milliseconds)
    pub max_idle_time_ms: u64,
    /// Maximum time to wait for a connection from the pool (in milliseconds)
    pub connection_timeout_ms: u64,
    /// Time to wait between connection attempts (in milliseconds)
    pub retry_delay_ms: u64,
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Concurrent request limit
    pub max_concurrent_requests: usize,
    /// Rate limiting: max requests per second
    pub max_requests_per_second: Option<f64>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 64,                         // 增加到2的幂次，更好的内存对齐
            min_idle: 8,                          // 减少最小空闲连接
            max_idle_time_ms: 120_000,            // 2分钟 - 进一步减少空闲时间
            connection_timeout_ms: 3_000,         // 减少连接超时到3秒
            retry_delay_ms: 10,                   // 减少重试延迟到10ms
            max_retries: 2,                       // 减少重试次数到2次
            max_concurrent_requests: 32,          // 增加并发请求限制到2的幂次
            max_requests_per_second: Some(100.0), // 增加速率限制
        }
    }
}

impl PoolConfig {
    /// Get max idle time as Duration
    pub const fn max_idle_time(&self) -> Duration {
        Duration::from_millis(self.max_idle_time_ms)
    }

    /// Get connection timeout as Duration
    pub const fn connection_timeout(&self) -> Duration {
        Duration::from_millis(self.connection_timeout_ms)
    }

    /// Get retry delay as Duration
    pub const fn retry_delay(&self) -> Duration {
        Duration::from_millis(self.retry_delay_ms)
    }
}

/// A pooled connection wrapper
pub struct PooledConnection {
    inner: Option<LocalSocketStream>,
    created_at: Instant,
    last_used: Instant,
    pool: Arc<ConnectionPoolInner>,
}

impl PooledConnection {
    fn new(stream: LocalSocketStream, pool: Arc<ConnectionPoolInner>) -> Self {
        let now = Instant::now();
        Self {
            inner: Some(stream),
            created_at: now,
            last_used: now,
            pool,
        }
    }

    /// Get the underlying stream
    pub fn stream(&mut self) -> Option<&mut LocalSocketStream> {
        self.last_used = Instant::now();
        self.inner.as_mut()
    }

    /// Take ownership of the underlying stream
    pub fn into_stream(mut self) -> Option<LocalSocketStream> {
        self.inner.take()
    }

    /// Check if connection is still valid
    pub fn is_valid(&self) -> bool {
        self.inner.is_some() && self.last_used.elapsed() < self.pool.config.max_idle_time()
    }

    /// Get connection age
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Get idle time
    pub fn idle_time(&self) -> Duration {
        self.last_used.elapsed()
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(stream) = self.inner.take() {
            self.pool.return_connection(stream);
        }
    }
}

/// Internal pool state with PUT request optimization
struct ConnectionPoolInner {
    name: Name<'static>,
    config: PoolConfig,
    connections: Mutex<VecDeque<(LocalSocketStream, Instant)>>,
    semaphore: Semaphore,
    /// 专用于PUT请求的新连接缓存
    fresh_connections: Mutex<VecDeque<LocalSocketStream>>,
    /// 快速路径计数器，用于避免semaphore竞争
    active_connections: std::sync::atomic::AtomicUsize,
}

impl ConnectionPoolInner {
    fn new(name: Name<'static>, config: PoolConfig) -> Self {
        Self {
            name,
            semaphore: Semaphore::new(config.max_size),
            connections: Mutex::new(VecDeque::new()),
            fresh_connections: Mutex::new(VecDeque::new()),
            active_connections: std::sync::atomic::AtomicUsize::new(0),
            config,
        }
    }

    /// Get a fresh connection for PUT requests, bypassing normal pool
    async fn get_fresh_connection(&self) -> Result<LocalSocketStream> {
        // 首先检查是否有预备的新连接
        {
            let mut fresh = self.fresh_connections.lock();
            if let Some(stream) = fresh.pop_front() {
                return Ok(stream);
            }
        }

        // 创建新连接，使用更短的超时和优化的参数
        let mut last_error = None;
        for attempt in 0..2 {
            // 只重试1次，更快失败
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await; // 很短的重试延迟
            }

            match LocalSocketStream::connect(self.name.clone()).await {
                Ok(stream) => {
                    debug!("Created fresh connection for PUT request");
                    return Ok(stream);
                }
                Err(e) => {
                    warn!("Fresh connection attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        // 如果新连接失败，回退到池化连接
        match self.get_pooled_connection() {
            Some(stream) => {
                debug!("Falling back to pooled connection for PUT request");
                Ok(stream)
            }
            None => Err(KodeBridgeError::connection(format!(
                "Failed to get fresh connection and no pooled connections available: {}",
                last_error
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Unknown error".to_string())
            ))),
        }
    }

    /// 预热新连接池，为PUT请求做准备
    async fn preheat_fresh_connections(&self, count: usize) {
        let mut successful = 0;
        for _ in 0..count {
            match LocalSocketStream::connect(self.name.clone()).await {
                Ok(stream) => {
                    self.fresh_connections.lock().push_back(stream);
                    successful += 1;
                }
                Err(_) => break,
            }
        }
        if successful > 0 {
            debug!("Preheated {} fresh connections", successful);
        }
    }

    async fn create_connection(&self) -> Result<LocalSocketStream> {
        let mut last_error = None;
        let mut delay = self.config.retry_delay();
        let max_delay = Duration::from_millis(200); // 限制最大延迟为200ms

        for attempt in 0..self.config.max_retries {
            if attempt > 0 {
                // 优化的指数退避，避免过长的延迟
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, max_delay);
            }

            match LocalSocketStream::connect(self.name.clone()).await {
                Ok(stream) => {
                    debug!("Created new connection on attempt {}", attempt + 1);
                    return Ok(stream);
                }
                Err(e) => {
                    warn!("Connection attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        Err(KodeBridgeError::connection(format!(
            "Failed to get fresh connection and no pooled connections available: {}",
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    fn get_pooled_connection(&self) -> Option<LocalSocketStream> {
        let mut connections = self.connections.lock();

        // Remove expired connections
        let now = Instant::now();
        while let Some((_, created_at)) = connections.front() {
            if now.duration_since(*created_at) > self.config.max_idle_time() {
                connections.pop_front();
            } else {
                break;
            }
        }

        // Get a connection if available
        connections.pop_front().map(|(stream, _)| {
            trace!("Reusing pooled connection, {} remaining", connections.len());
            stream
        })
    }

    fn return_connection(&self, stream: LocalSocketStream) {
        // 减少活跃连接计数
        self.active_connections
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

        let (kept, pool_size) = {
            let mut connections = self.connections.lock();

            // Only keep the connection if we haven't exceeded max_size
            if connections.len() < self.config.max_size {
                connections.push_back((stream, Instant::now()));
                (true, connections.len())
            } else {
                (false, connections.len())
            }
        };

        if kept {
            trace!("Returned connection to pool, {} total", pool_size);
        } else {
            trace!("Pool full, dropping connection");
        }
    }

    async fn get_connection_with_timeout(&self) -> Result<LocalSocketStream> {
        // 优化的获取连接逻辑，减少semaphore竞争

        // 首先快速检查是否有可用的池化连接
        if let Some(stream) = self.get_pooled_connection() {
            return Ok(stream);
        }

        // 检查活跃连接数，避免不必要的semaphore等待
        let active_count = self
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed);
        if active_count >= self.config.max_size {
            // 快速失败路径，避免长时间等待
            return Err(KodeBridgeError::custom("Connection pool exhausted"));
        }

        // 使用更短的超时来获取许可
        let timeout = std::cmp::min(self.config.connection_timeout(), Duration::from_millis(500));
        let permit = tokio::time::timeout(timeout, self.semaphore.acquire())
            .await
            .map_err(|_| KodeBridgeError::timeout(timeout.as_millis() as u64))?
            .map_err(|_| KodeBridgeError::custom("Semaphore closed"))?;

        // 再次检查池化连接（避免不必要的连接创建）
        if let Some(stream) = self.get_pooled_connection() {
            drop(permit);
            return Ok(stream);
        }

        // 增加活跃连接计数
        self.active_connections
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // 创建新连接
        match self.create_connection().await {
            Ok(stream) => {
                drop(permit);
                Ok(stream)
            }
            Err(e) => {
                // 出错时减少活跃连接计数
                self.active_connections
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                drop(permit);
                Err(e)
            }
        }
    }

    /// Get a fresh connection optimized for PUT requests
    async fn get_fresh_connection_with_timeout(&self) -> Result<LocalSocketStream> {
        // 对PUT请求使用专门优化的逻辑
        let permit = tokio::time::timeout(Duration::from_millis(100), self.semaphore.acquire())
            .await
            .map_err(|_| KodeBridgeError::timeout(100))?
            .map_err(|_| KodeBridgeError::custom("Semaphore closed"))?;

        // Get fresh connection directly with optimized parameters
        let stream = self.get_fresh_connection().await?;
        drop(permit);
        Ok(stream)
    }
}

/// High-performance connection pool for IPC connections
#[derive(Clone)]
pub struct ConnectionPool {
    inner: Arc<ConnectionPoolInner>,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new(name: Name<'static>, config: PoolConfig) -> Self {
        Self {
            inner: Arc::new(ConnectionPoolInner::new(name, config)),
        }
    }

    /// Create a connection pool with default configuration
    pub fn with_default_config(name: Name<'static>) -> Self {
        Self::new(name, PoolConfig::default())
    }

    /// Get a connection from the pool
    pub async fn get_connection(&self) -> Result<PooledConnection> {
        let stream = self.inner.get_connection_with_timeout().await?;
        Ok(PooledConnection::new(stream, Arc::clone(&self.inner)))
    }

    /// Get a fresh connection optimized for PUT requests
    pub async fn get_fresh_connection(&self) -> Result<PooledConnection> {
        let stream = self.inner.get_fresh_connection_with_timeout().await?;
        Ok(PooledConnection::new(stream, Arc::clone(&self.inner)))
    }

    /// Preheat fresh connections for better PUT performance
    pub async fn preheat_for_puts(&self, count: usize) {
        self.inner.preheat_fresh_connections(count).await;
    }

    /// Get multiple connections for concurrent operations
    pub async fn get_connections(&self, count: usize) -> Result<Vec<PooledConnection>> {
        let mut connections = Vec::with_capacity(count);

        // Use semaphore to control concurrent acquisition
        let mut tasks = Vec::new();
        for _ in 0..count {
            let pool = self.clone();
            tasks.push(tokio::spawn(async move { pool.get_connection().await }));
        }

        // Wait for all connection acquisitions to complete
        for task in tasks {
            match task.await {
                Ok(Ok(conn)) => connections.push(conn),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(KodeBridgeError::custom(format!("Task failed: {}", e))),
            }
        }

        Ok(connections)
    }

    /// Get pool statistics
    pub fn stats(&self) -> PoolStats {
        let connections = self.inner.connections.lock();
        let active_count = self
            .inner
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed);
        PoolStats {
            total_connections: connections.len(),
            available_permits: self.inner.semaphore.available_permits(),
            max_size: self.inner.config.max_size,
            active_connections: active_count,
        }
    }

    /// Close all pooled connections
    pub fn close(&self) {
        self.inner.connections.lock().clear();
        debug!("Closed all pooled connections");
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total_connections: usize,
    pub available_permits: usize,
    pub max_size: usize,
    pub active_connections: usize,
}

impl std::fmt::Display for PoolStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pool(connections: {}, active: {}, permits: {}, max: {})",
            self.total_connections, self.active_connections, self.available_permits, self.max_size
        )
    }
}
