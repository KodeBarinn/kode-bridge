use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName as _};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::errors::{KodeBridgeError, Result};
use crate::http_client::RequestBuilder;
use crate::stream_client::{send_streaming_request, StreamingResponse};
use http::Method;
use std::str::FromStr as _;
use tracing::{debug, trace};

/// Configuration for IPC streaming client
#[derive(Debug, Clone)]
pub struct StreamClientConfig {
    /// Default timeout for connections
    pub default_timeout: Duration,
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Delay between retry attempts
    pub retry_delay: Duration,
    /// Buffer size for streaming
    pub buffer_size: usize,
}

impl Default for StreamClientConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(60),
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            buffer_size: 8192,
        }
    }
}

/// Specialized IPC streaming client for handling real-time data streams
pub struct IpcStreamClient {
    name: Name<'static>,
    config: StreamClientConfig,
}

/// Stream request builder for fluent API
pub struct StreamRequestBuilder<'a> {
    client: &'a IpcStreamClient,
    method: Method,
    path: String,
    body: Option<Value>,
    timeout: Duration,
    headers: Vec<(String, String)>,
}

/// Stream response wrapper with chainable methods
pub struct StreamResponse {
    inner: StreamingResponse,
}

impl StreamResponse {
    const fn new(response: StreamingResponse) -> Self {
        Self { inner: response }
    }

    /// Get the HTTP status code
    pub const fn status(&self) -> u16 {
        self.inner.status_code()
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

    /// Convert stream to JSON objects with automatic parsing and timeout
    pub async fn json_results<T>(self) -> Result<Vec<T>>
    where
        T: DeserializeOwned + Send,
    {
        self.inner.json(Duration::from_secs(30)).await
    }

    /// Convert stream to JSON objects with custom timeout
    pub async fn json<T>(self, timeout: Duration) -> Result<Vec<T>>
    where
        T: DeserializeOwned + Send,
    {
        self.inner.json(timeout).await
    }

    /// Process stream in real-time with a handler
    pub async fn process_lines<F>(self, timeout: Duration, mut handler: F) -> Result<()>
    where
        F: FnMut(&str) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> + Send,
    {
        self.inner
            .process_lines_with_timeout(timeout, |line| {
                handler(line).map(|_| true) // Convert to continue flag
            })
            .await
    }

    /// Process stream with custom JSON processing
    pub async fn process_json<F, T>(self, timeout: Duration, handler: F) -> Result<Vec<T>>
    where
        F: FnMut(&str) -> Option<T> + Send,
        T: Send + 'static,
    {
        self.inner.process_json(timeout, handler).await
    }

    /// Collect all stream content as text
    pub async fn collect_text(self) -> Result<String> {
        self.inner.collect_text().await
    }

    /// Collect stream content with timeout
    pub async fn collect_text_with_timeout(self, timeout: Duration) -> Result<String> {
        self.inner.collect_text_with_timeout(timeout).await
    }

    /// Get the underlying modern StreamingResponse
    pub fn into_inner(self) -> StreamingResponse {
        self.inner
    }

    /// Get headers as JSON for backward compatibility
    pub fn headers(&self) -> Value {
        self.inner.headers_json()
    }
}

impl IpcStreamClient {
    /// Create a new IPC streaming client with default configuration
    pub fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        Self::with_config(path, StreamClientConfig::default())
    }

    /// Create a new IPC streaming client with custom configuration
    pub fn with_config<P>(path: P, config: StreamClientConfig) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let name = path
            .as_ref()
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| KodeBridgeError::configuration(format!("Invalid path: {}", e)))?
            .into_owned();

        Ok(Self { name, config })
    }

    /// Create a connection with retry logic
    async fn create_connection(&self) -> Result<LocalSocketStream> {
        let mut last_error = None;

        for attempt in 0..self.config.max_retries {
            if attempt > 0 {
                tokio::time::sleep(self.config.retry_delay).await;
            }

            match LocalSocketStream::connect(self.name.clone()).await {
                Ok(stream) => {
                    debug!("Created streaming connection on attempt {}", attempt + 1);
                    return Ok(stream);
                }
                Err(e) => {
                    trace!("Streaming connection attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);
                }
            }
        }

        Err(KodeBridgeError::connection(format!(
            "Failed to create streaming connection after {} attempts: {}",
            self.config.max_retries,
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    /// Internal method to send streaming requests
    async fn send_request_internal(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<StreamingResponse> {
        let method = Method::from_str(method)
            .map_err(|e| KodeBridgeError::invalid_request(format!("Invalid method: {}", e)))?;

        let mut builder = RequestBuilder::new(method, path.to_string());

        if let Some(json_body) = body {
            builder = builder.json(json_body)?;
        }

        let request = builder.build()?;

        // Execute with timeout
        let result = tokio::time::timeout(timeout, async {
            let stream = self.create_connection().await?;
            send_streaming_request(stream, request).await
        })
        .await;

        match result {
            Ok(response) => response,
            Err(_) => Err(KodeBridgeError::timeout(timeout.as_millis() as u64)),
        }
    }

    /// GET streaming request
    pub fn get(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::GET, path)
    }

    /// POST streaming request
    pub fn post(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::POST, path)
    }

    /// PUT streaming request
    pub fn put(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::PUT, path)
    }

    /// DELETE streaming request
    pub fn delete(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::DELETE, path)
    }

    /// PATCH streaming request
    pub fn patch(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::PATCH, path)
    }

    /// HEAD streaming request
    pub fn head(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::HEAD, path)
    }

    /// OPTIONS streaming request
    pub fn options(&self, path: &str) -> StreamRequestBuilder<'_> {
        StreamRequestBuilder::new(self, Method::OPTIONS, path)
    }
}

impl<'a> StreamRequestBuilder<'a> {
    fn new(client: &'a IpcStreamClient, method: Method, path: &str) -> Self {
        Self {
            client,
            method,
            path: path.to_string(),
            body: None,
            timeout: client.config.default_timeout,
            headers: Vec::new(),
        }
    }

    /// Set JSON body
    pub fn json_body(mut self, body: &Value) -> Self {
        self.body = Some(body.clone());
        self
    }

    /// Set custom timeout
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
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

    /// Send the streaming request
    pub async fn send(self) -> Result<StreamResponse> {
        let response = self
            .client
            .send_request_internal(
                self.method.as_str(),
                &self.path,
                self.body.as_ref(),
                self.timeout,
            )
            .await?;

        Ok(StreamResponse::new(response))
    }

    /// Send and get JSON results automatically (convenience method)
    pub async fn json_results<T>(self) -> Result<Vec<T>>
    where
        T: DeserializeOwned + Send,
    {
        let response = self.send().await?;
        response.json_results().await
    }

    /// Send and process lines with a handler (convenience method)
    pub async fn process_lines<F>(self, handler: F) -> Result<()>
    where
        F: FnMut(&str) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> + Send,
    {
        let timeout = self.timeout;
        let response = self.send().await?;
        response.process_lines(timeout, handler).await
    }
}
