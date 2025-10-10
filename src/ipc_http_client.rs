use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::errors::{KodeBridgeError, Result};
use crate::http_client::{send_request, RequestBuilder, Response};
use crate::metrics::global_metrics;
use crate::pool::{ConnectionPool, PoolConfig, PooledConnection};
use crate::retry::{RetryConfig, RetryExecutor};
use http::Method;
use std::str::FromStr;
use tracing::{debug, trace};

/// Configuration for IPC HTTP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Default timeout for requests
    pub default_timeout: Duration,
    /// Connection pool configuration
    pub pool_config: PoolConfig,
    /// Enable connection pooling
    pub enable_pooling: bool,
    /// Retry configuration
    pub max_retries: usize,
    pub retry_delay: Duration,
    /// Concurrent request limit
    pub max_concurrent_requests: usize,
    /// Rate limiting: max requests per second
    pub max_requests_per_second: Option<f64>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(5), // 减少默认超时到5秒
            pool_config: PoolConfig::default(),
            enable_pooling: true,
            max_retries: 3,                         // 减少重试次数
            retry_delay: Duration::from_millis(25), // 减少重试延迟
            max_concurrent_requests: 16,            // 增加并发请求数
            max_requests_per_second: Some(50.0),    // 增加请求速率限制
        }
    }
}

/// Generic IPC HTTP client that works on both Unix and Windows platforms
///
/// This client is optimized for request-response patterns with connection pooling support.
/// For streaming functionality, use `IpcStreamClient` instead.
pub struct IpcHttpClient {
    name: Name<'static>,
    config: ClientConfig,
    pool: Option<ConnectionPool>,
    retry_executor: RetryExecutor,
    /// 专门用于PUT请求的重试执行器
    put_retry_executor: RetryExecutor,
}

/// HTTP request builder for fluent API
pub struct HttpRequestBuilder<'a> {
    client: &'a IpcHttpClient,
    method: Method,
    path: String,
    body: Option<Value>,
    timeout: Option<Duration>,
    headers: Vec<(String, String)>,
    /// PUT专用优化标志
    put_optimized: bool,
    /// 预期数据大小，用于选择合适的缓冲区和超时
    expected_size: Option<usize>,
}

/// Enhanced HTTP response wrapper with chainable methods
#[derive(Debug)]
pub struct HttpResponse {
    inner: Response,
}

impl HttpResponse {
    fn new(response: Response) -> Self {
        Self { inner: response }
    }

    /// Get the HTTP status code
    pub fn status(&self) -> u16 {
        self.inner.status_code()
    }

    /// Get response headers as JSON value (for backward compatibility)
    pub fn headers(&self) -> Value {
        let headers_map: std::collections::HashMap<String, String> = self
            .inner
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        serde_json::to_value(headers_map).unwrap_or(Value::Null)
    }

    /// Get response body as string
    pub fn body(&self) -> Result<String> {
        self.inner.text()
    }

    /// Check if response indicates success (2xx status)
    pub fn is_success(&self) -> bool {
        self.inner.is_success()
    }

    /// Check if response indicates client error (4xx status)
    pub fn is_client_error(&self) -> bool {
        self.inner.is_client_error()
    }

    /// Check if response indicates server error (5xx status)
    pub fn is_server_error(&self) -> bool {
        self.inner.is_server_error()
    }

    /// Get content length from headers
    pub fn content_length(&self) -> u64 {
        self.inner.content_length().unwrap_or(0)
    }

    /// Parse response body as JSON
    pub fn json<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.inner.json()
    }

    /// Parse response body as generic JSON value
    pub fn json_value(&self) -> Result<Value> {
        self.inner.json_value()
    }

    /// Get the underlying modern Response
    pub fn into_inner(self) -> Response {
        self.inner
    }

    /// Convert to legacy Response format for backward compatibility
    pub fn to_legacy(&self) -> crate::response::LegacyResponse {
        self.inner.to_legacy()
    }
}

impl IpcHttpClient {
    /// Create a new IPC HTTP client with default configuration
    pub fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        Self::with_config(path, ClientConfig::default())
    }

    /// Create a new IPC HTTP client with custom configuration
    pub fn with_config<P>(path: P, config: ClientConfig) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let name = path
            .as_ref()
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| KodeBridgeError::configuration(format!("Invalid path: {}", e)))?
            .into_owned();

        let pool = if config.enable_pooling {
            Some(ConnectionPool::new(
                name.clone(),
                config.pool_config.clone(),
            ))
        } else {
            None
        };

        // Create retry executor with optimized configuration for different request types
        let retry_config = RetryConfig::for_network_operations()
            .max_attempts(config.max_retries)
            .base_delay(Duration::from_millis(config.pool_config.retry_delay_ms));

        let retry_executor = RetryExecutor::new(retry_config);

        // 创建专门用于PUT请求的快速重试执行器
        let put_retry_config = RetryConfig::for_put_requests();
        let put_retry_executor = RetryExecutor::new(put_retry_config);

        Ok(Self {
            name,
            config,
            pool,
            retry_executor,
            put_retry_executor,
        })
    }

    /// Create a direct connection (bypassing pool)
    async fn create_direct_connection(&self) -> Result<LocalSocketStream> {
        let mut last_error = None;

        for attempt in 0..self.config.max_retries {
            if attempt > 0 {
                tokio::time::sleep(self.config.retry_delay).await;
            }

            match LocalSocketStream::connect(self.name.clone()).await {
                Ok(stream) => {
                    debug!("Created direct connection on attempt {}", attempt + 1);
                    return Ok(stream);
                }
                Err(e) => {
                    trace!("Connection attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        Err(KodeBridgeError::connection(format!(
            "Failed to create connection after {} attempts: {}",
            self.config.max_retries,
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    /// Get a connection (from pool or create new)
    async fn get_connection(&self) -> Result<Either<PooledConnection, LocalSocketStream>> {
        let metrics = global_metrics();

        if let Some(ref pool) = self.pool {
            match pool.get_connection().await {
                Ok(conn) => {
                    metrics.connection_created(true); // From pool
                    Ok(Either::Pool(conn))
                }
                Err(e) => {
                    metrics.connection_failed();
                    Err(e)
                }
            }
        } else {
            match self.create_direct_connection().await {
                Ok(stream) => {
                    metrics.connection_created(false); // Direct connection
                    Ok(Either::Direct(stream))
                }
                Err(e) => {
                    metrics.connection_failed();
                    Err(e)
                }
            }
        }
    }

    /// Legacy request method for backward compatibility
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> crate::errors::AnyResult<crate::response::LegacyResponse> {
        let response = self
            .send_request_internal(method, path, body, self.config.default_timeout)
            .await?;
        Ok(response.to_legacy())
    }

    /// Internal method to send requests with enhanced retry logic
    async fn send_request_internal(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let method = Method::from_str(method)
            .map_err(|e| KodeBridgeError::invalid_request(format!("Invalid method: {}", e)))?;

        let mut builder = RequestBuilder::new(method.clone(), path.to_string());

        // Note: This method doesn't support custom headers for backward compatibility
        // Use the fluent API (get(), post(), etc.) for custom headers

        if let Some(json_body) = body {
            builder = builder.json(json_body)?;
        }

        let request = builder.build()?;

        // Use smart retry mechanism
        self.retry_executor
            .execute_with_context(&format!("{} {}", method.as_str(), path), || async {
                // Execute with timeout
                let result = tokio::time::timeout(timeout, async {
                    let mut connection = self.get_connection().await?;

                    match &mut connection {
                        Either::Pool(conn) => {
                            if let Some(stream) = conn.stream() {
                                send_request(stream, request.clone()).await
                            } else {
                                Err(KodeBridgeError::connection("Pooled connection is invalid"))
                            }
                        }
                        Either::Direct(stream) => send_request(stream, request.clone()).await,
                    }
                })
                .await;

                match result {
                    Ok(response) => response,
                    Err(_) => Err(KodeBridgeError::timeout(timeout.as_millis() as u64)),
                }
            })
            .await
    }

    /// Enhanced request sending with PUT optimization support
    async fn send_request_with_optimization(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        headers: &[(String, String)],
        timeout: Duration,
        is_put_optimized: bool,
        expected_size: Option<usize>,
    ) -> Result<Response> {
        let method_enum = Method::from_str(method)
            .map_err(|e| KodeBridgeError::invalid_request(format!("Invalid method: {}", e)))?;

        let mut builder = RequestBuilder::new(method_enum.clone(), path.to_string());

        // Add custom headers
        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        if let Some(json_body) = body {
            builder = builder.json(json_body)?;
        }

        let request = builder.build()?;

        // PUT请求使用专门的重试策略
        let retry_context = if is_put_optimized {
            format!("PUT_OPTIMIZED {}", path)
        } else {
            format!("{} {}", method, path)
        };

        // Use smart retry mechanism with PUT optimization
        let retry_executor = if is_put_optimized {
            &self.put_retry_executor // 使用PUT专用的快速重试器
        } else {
            &self.retry_executor // 使用通用重试器
        };

        retry_executor
            .execute_with_context(&retry_context, || async {
                // Execute with timeout
                let result = tokio::time::timeout(timeout, async {
                    let mut connection = if is_put_optimized && expected_size.unwrap_or(0) > 10240 {
                        // 对于大的PUT请求，优先获取新连接
                        self.get_fresh_connection().await?
                    } else {
                        self.get_connection().await?
                    };

                    match &mut connection {
                        Either::Pool(conn) => {
                            if let Some(stream) = conn.stream() {
                                send_request(stream, request.clone()).await
                            } else {
                                Err(KodeBridgeError::connection("Pooled connection is invalid"))
                            }
                        }
                        Either::Direct(stream) => send_request(stream, request.clone()).await,
                    }
                })
                .await;

                match result {
                    Ok(response) => response,
                    Err(_) => Err(KodeBridgeError::timeout(timeout.as_millis() as u64)),
                }
            })
            .await
    }

    /// Get a fresh connection optimized for PUT requests
    async fn get_fresh_connection(&self) -> Result<Either<PooledConnection, LocalSocketStream>> {
        use interprocess::local_socket::tokio::prelude::LocalSocketStream;

        // 首先尝试从连接池获取新连接
        if let Some(ref pool) = self.pool {
            match tokio::time::timeout(Duration::from_millis(20), pool.get_fresh_connection()).await
            {
                Ok(Ok(conn)) => return Ok(Either::Pool(conn)),
                Ok(Err(_)) | Err(_) => {
                    // 池化新连接失败，继续尝试直接连接
                }
            }
        }

        // 直接创建连接，使用更快的超时设置
        match tokio::time::timeout(
            Duration::from_millis(100),
            LocalSocketStream::connect(self.name.clone()),
        )
        .await
        {
            Ok(Ok(stream)) => Ok(Either::Direct(stream)),
            Ok(Err(_)) | Err(_) => {
                // 如果直接连接失败，回退到普通池化连接
                if let Some(ref pool) = self.pool {
                    let conn = pool.get_connection().await?;
                    Ok(Either::Pool(conn))
                } else {
                    Err(KodeBridgeError::connection(
                        "Failed to get fresh connection",
                    ))
                }
            }
        }
    }

    /// GET request
    pub fn get(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::GET, path)
    }

    /// POST request
    pub fn post(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::POST, path)
    }

    /// PUT request with optimization enabled by default
    pub fn put(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::PUT, path)
    }

    /// Optimized batch PUT operations
    pub async fn put_batch(&self, requests: Vec<(String, Value)>) -> Result<Vec<HttpResponse>> {
        let batch_size = requests.len();
        if batch_size == 0 {
            return Ok(Vec::new());
        }

        // 限制并发数以避免过载
        let concurrent_limit = std::cmp::min(self.config.max_concurrent_requests, batch_size);
        let mut responses = Vec::with_capacity(batch_size);

        // 分批处理以控制内存使用和网络负载
        for chunk in requests.chunks(concurrent_limit) {
            let mut futures = Vec::new();

            for (path, body) in chunk {
                let path = path.clone();
                let body = body.clone();

                let future = self.put(&path).json_body(&body).optimize_for_put().send();

                futures.push(future);
            }

            // 并发等待当前批次完成
            let chunk_results = futures::future::join_all(futures).await;

            for result in chunk_results {
                match result {
                    Ok(response) => responses.push(response),
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(responses)
    }

    /// DELETE request
    pub fn delete(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::DELETE, path)
    }

    /// PATCH request
    pub fn patch(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::PATCH, path)
    }

    /// HEAD request
    pub fn head(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::HEAD, path)
    }

    /// OPTIONS request
    pub fn options(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::OPTIONS, path)
    }

    /// Get pool statistics (if pooling is enabled)
    pub fn pool_stats(&self) -> Option<crate::pool::PoolStats> {
        self.pool.as_ref().map(|p| p.stats())
    }

    /// Close the client and clean up resources
    pub fn close(&self) {
        if let Some(ref pool) = self.pool {
            pool.close();
        }
    }

    /// Preheat connections for better PUT performance
    pub async fn preheat_for_puts(&self, count: usize) {
        if let Some(ref pool) = self.pool {
            pool.preheat_for_puts(count).await;
        }
    }

    /// Smart timeout calculation based on request characteristics
    fn calculate_smart_timeout(&self, method: &str, body_size: Option<usize>) -> Duration {
        match method {
            "PUT" | "POST" => {
                match body_size {
                    Some(size) if size > 5 * 1024 * 1024 => Duration::from_secs(30), // >5MB: 30s
                    Some(size) if size > 1024 * 1024 => Duration::from_secs(15),     // >1MB: 15s
                    Some(size) if size > 100 * 1024 => Duration::from_secs(8),       // >100KB: 8s
                    Some(size) if size > 10 * 1024 => Duration::from_secs(4),        // >10KB: 4s
                    _ => Duration::from_secs(2),                                     // 小请求: 2s
                }
            }
            _ => self.config.default_timeout, // 其他方法使用默认超时
        }
    }
}

impl<'a> HttpRequestBuilder<'a> {
    fn new(client: &'a IpcHttpClient, method: Method, path: &str) -> Self {
        let is_put = method == Method::PUT;
        Self {
            client,
            method,
            path: path.to_string(),
            body: None,
            timeout: None,
            headers: Vec::new(),
            put_optimized: is_put, // 自动为PUT请求启用优化
            expected_size: None,
        }
    }

    /// Set JSON body
    pub fn json_body(mut self, body: &Value) -> Self {
        self.body = Some(body.clone());

        // 为PUT请求估算数据大小以优化处理
        if self.method == Method::PUT {
            if let Ok(json_bytes) = serde_json::to_vec(body) {
                self.expected_size = Some(json_bytes.len());
            }
        }

        self
    }

    /// Set custom timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// 设置预期数据大小（用于PUT请求优化）
    pub fn expected_size(mut self, size: usize) -> Self {
        self.expected_size = Some(size);
        self
    }

    /// 启用PUT请求专门优化
    pub fn optimize_for_put(mut self) -> Self {
        self.put_optimized = true;
        self
    }

    /// Add custom header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.headers.push((key.into(), value.into()));
        self
    }

    /// Send the request
    pub async fn send(self) -> Result<HttpResponse> {
        let metrics = global_metrics();
        let tracker = metrics.request_start(self.method.as_str());

        // 为PUT请求优化超时设置，使用智能超时计算
        let timeout = if self.put_optimized {
            self.timeout.unwrap_or_else(|| {
                self.client
                    .calculate_smart_timeout(self.method.as_str(), self.expected_size)
            })
        } else {
            self.timeout.unwrap_or(self.client.config.default_timeout)
        };

        match self
            .client
            .send_request_with_optimization(
                self.method.as_str(),
                &self.path,
                self.body.as_ref(),
                &self.headers,
                timeout,
                self.put_optimized,
                self.expected_size,
            )
            .await
        {
            Ok(response) => {
                tracker.success(response.status_code());
                Ok(HttpResponse::new(response))
            }
            Err(e) => {
                tracker.failure(&format!("{:?}", e));
                Err(e)
            }
        }
    }
}

/// Helper enum for connection types
enum Either<A, B> {
    Pool(A),
    Direct(B),
}

impl Drop for IpcHttpClient {
    fn drop(&mut self) {
        self.close();
    }
}
