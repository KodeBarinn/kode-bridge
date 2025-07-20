use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::errors::{KodeBridgeError, Result};
use crate::http_client::{RequestBuilder, Response, send_request};
use crate::pool::{ConnectionPool, PoolConfig, PooledConnection};
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
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            pool_config: PoolConfig::default(),
            enable_pooling: true,
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
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
}

/// HTTP request builder for fluent API
pub struct HttpRequestBuilder<'a> {
    client: &'a IpcHttpClient,
    method: Method,
    path: String,
    body: Option<Value>,
    timeout: Option<Duration>,
    headers: Vec<(String, String)>,
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

        Ok(Self { name, config, pool })
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
            last_error.unwrap()
        )))
    }

    /// Get a connection (from pool or create new)
    async fn get_connection(&self) -> Result<Either<PooledConnection, LocalSocketStream>> {
        if let Some(ref pool) = self.pool {
            pool.get_connection().await.map(Either::Pool)
        } else {
            self.create_direct_connection().await.map(Either::Direct)
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

    /// Internal method to send requests
    async fn send_request_internal(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Response> {
        let method = Method::from_str(method)
            .map_err(|e| KodeBridgeError::invalid_request(format!("Invalid method: {}", e)))?;

        let mut builder = RequestBuilder::new(method, path.to_string());

        if let Some(json_body) = body {
            builder = builder.json(json_body)?;
        }

        let request = builder.build()?;

        // Execute with timeout
        let result = tokio::time::timeout(timeout, async {
            let mut connection = self.get_connection().await?;

            match &mut connection {
                Either::Pool(conn) => {
                    if let Some(stream) = conn.stream() {
                        send_request(stream, request).await
                    } else {
                        Err(KodeBridgeError::connection("Pooled connection is invalid"))
                    }
                }
                Either::Direct(stream) => send_request(stream, request).await,
            }
        })
        .await;

        match result {
            Ok(response) => response,
            Err(_) => Err(KodeBridgeError::timeout(timeout.as_millis() as u64)),
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

    /// PUT request
    pub fn put(&self, path: &str) -> HttpRequestBuilder<'_> {
        HttpRequestBuilder::new(self, Method::PUT, path)
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
}

impl<'a> HttpRequestBuilder<'a> {
    fn new(client: &'a IpcHttpClient, method: Method, path: &str) -> Self {
        Self {
            client,
            method,
            path: path.to_string(),
            body: None,
            timeout: None,
            headers: Vec::new(),
        }
    }

    /// Set JSON body
    pub fn json_body(mut self, body: &Value) -> Self {
        self.body = Some(body.clone());
        self
    }

    /// Set custom timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
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
        let timeout = self.timeout.unwrap_or(self.client.config.default_timeout);
        let response = self
            .client
            .send_request_internal(
                self.method.as_str(),
                &self.path,
                self.body.as_ref(),
                timeout,
            )
            .await?;

        Ok(HttpResponse::new(response))
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
