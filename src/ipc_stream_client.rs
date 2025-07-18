use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::AnyResult;
use crate::stream_client::{StreamingResponse, http_request_stream};

/// Specialized IPC streaming client for handling real-time data streams
pub struct IpcStreamClient {
    name: Name<'static>,
    default_timeout: Duration,
}

/// Stream request builder for fluent API
pub struct StreamRequestBuilder<'a> {
    client: &'a IpcStreamClient,
    method: String,
    path: String,
    body: Option<serde_json::Value>,
    timeout: Duration,
}

/// Stream response wrapper with chainable methods
pub struct StreamResponse {
    inner: StreamingResponse,
}

impl StreamResponse {
    fn new(response: StreamingResponse) -> Self {
        Self { inner: response }
    }

    /// Get the HTTP status code
    pub fn status(&self) -> u16 {
        self.inner.status
    }

    /// Convert stream to JSON objects
    pub async fn json<T>(self, timeout: Duration) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send,
    {
        self.inner.json(timeout).await
    }

    /// Process stream in real-time with a handler
    pub async fn process_lines<F>(self, timeout: Duration, handler: F) -> AnyResult<()>
    where
        F: FnMut(&str) -> bool,
    {
        self.inner.process_realtime(timeout, handler).await
    }

    /// Collect all lines as strings
    pub async fn lines(self, timeout: Duration) -> AnyResult<Vec<String>> {
        let mut lines = Vec::new();
        self.inner
            .process_realtime(timeout, |line| {
                if !line.trim().is_empty() {
                    lines.push(line.to_string());
                }
                true
            })
            .await?;
        Ok(lines)
    }

    /// Get first N items from stream
    pub async fn take<T>(self, count: usize, timeout: Duration) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send,
    {
        let mut items = Vec::new();
        let mut collected = 0;

        self.inner
            .process_realtime(timeout, |line| {
                if collected >= count {
                    return false;
                }

                if let Ok(item) = serde_json::from_str::<T>(line) {
                    items.push(item);
                    collected += 1;
                }

                collected < count
            })
            .await?;

        Ok(items)
    }
}

impl<'a> StreamRequestBuilder<'a> {
    fn new(client: &'a IpcStreamClient, method: &str, path: &str) -> Self {
        Self {
            client,
            method: method.to_string(),
            path: path.to_string(),
            body: None,
            timeout: client.default_timeout,
        }
    }

    /// Set request body as JSON
    pub fn json(mut self, body: &serde_json::Value) -> Self {
        self.body = Some(body.clone());
        self
    }

    /// Set request timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Execute the request and return a stream response
    pub async fn send(self) -> AnyResult<StreamResponse> {
        let response = self
            .client
            .request_stream(&self.method, &self.path, self.body.as_ref())
            .await?;
        Ok(StreamResponse::new(response))
    }

    /// Execute request and directly get JSON results
    pub async fn json_results<T>(self) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send,
    {
        let timeout = self.timeout;
        let response = self.send().await?;
        response.json(timeout).await
    }

    /// Execute request and process lines in real-time
    pub async fn process_lines<F>(self, handler: F) -> AnyResult<()>
    where
        F: FnMut(&str) -> bool,
    {
        let timeout = self.timeout;
        let response = self.send().await?;
        response.process_lines(timeout, handler).await
    }
}

impl IpcStreamClient {
    /// Create a new IPC streaming client with the given socket/named pipe path
    ///
    /// # Arguments
    /// * `path` - The path to the socket (Unix) or named pipe (Windows)
    pub fn new<P: AsRef<Path>>(path: P) -> AnyResult<Self> {
        let name = path.as_ref().to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Self {
            name,
            default_timeout: Duration::from_secs(5), // Default 5 seconds timeout
        })
    }

    /// Create a new client with custom default timeout
    pub fn with_timeout<P: AsRef<Path>>(path: P, timeout: Duration) -> AnyResult<Self> {
        let name = path.as_ref().to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Self {
            name,
            default_timeout: timeout,
        })
    }

    /// HTTP-like GET request
    pub fn get(&self, path: &str) -> StreamRequestBuilder {
        StreamRequestBuilder::new(self, "GET", path)
    }

    /// HTTP-like POST request
    pub fn post(&self, path: &str) -> StreamRequestBuilder {
        StreamRequestBuilder::new(self, "POST", path)
    }

    /// HTTP-like PUT request
    pub fn put(&self, path: &str) -> StreamRequestBuilder {
        StreamRequestBuilder::new(self, "PUT", path)
    }

    /// HTTP-like DELETE request
    pub fn delete(&self, path: &str) -> StreamRequestBuilder {
        StreamRequestBuilder::new(self, "DELETE", path)
    }

    /// Generic request with custom method
    pub fn request(&self, method: &str, path: &str) -> StreamRequestBuilder {
        StreamRequestBuilder::new(self, method, path)
    }

    /// Low-level method: Send a streaming HTTP request over the IPC connection
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `path` - Request path
    /// * `body` - Optional JSON body
    pub async fn request_stream(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<StreamingResponse> {
        let stream = LocalSocketStream::connect(self.name.clone()).await?;
        http_request_stream(stream, method, path, body).await
    }    // =================== Backward Compatible Methods ===================

    /// Convenience method: Get JSON stream data (backward compatible)
    ///
    /// # Arguments
    /// * `path` - Request path
    /// * `timeout` - Timeout duration for collecting data
    pub async fn get_json_stream<T>(&self, path: &str, timeout: Duration) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send,
    {
        self.get(path).timeout(timeout).json_results().await
    }

    /// Convenience method: Process stream data in real-time (backward compatible)
    ///
    /// # Arguments
    /// * `path` - Request path
    /// * `timeout` - Timeout duration for processing
    /// * `handler` - Closure to handle each line of data
    pub async fn process_stream<F>(
        &self,
        path: &str,
        timeout: Duration,
        handler: F,
    ) -> AnyResult<()>
    where
        F: FnMut(&str) -> bool, // Return false to stop processing
    {
        self.get(path).timeout(timeout).process_lines(handler).await
    }
}
