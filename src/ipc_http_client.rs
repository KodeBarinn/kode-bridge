use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::errors::AnyResult;
use crate::http_client::Response;
use crate::http_client::http_request;

/// Generic IPC HTTP client that works on both Unix and Windows platforms
///
/// This client is optimized for request-response patterns, not streaming.
/// For streaming functionality, use `IpcStreamClient` instead.
pub struct IpcHttpClient {
    name: Name<'static>,
    #[allow(dead_code)]
    default_timeout: Duration,
}

/// HTTP request builder for fluent API
pub struct HttpRequestBuilder<'a> {
    client: &'a IpcHttpClient,
    method: String,
    path: String,
    body: Option<Value>,
    timeout: Option<Duration>,
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
        self.inner.status
    }

    /// Get response headers
    pub fn headers(&self) -> &Value {
        &self.inner.headers
    }

    /// Get response body as string
    pub fn text(&self) -> &str {
        &self.inner.body
    }

    /// Parse response body as JSON
    pub fn json<T>(&self) -> AnyResult<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(&self.inner.body).map_err(Into::into)
    }

    /// Parse response body as generic JSON value
    pub fn json_value(&self) -> AnyResult<Value> {
        self.inner.json()
    }

    /// Get response body length
    pub fn content_length(&self) -> usize {
        self.inner.body.len()
    }

    /// Convert to original Response type for backward compatibility
    pub fn into_inner(self) -> Response {
        self.inner
    }

    /// Check if response is successful (status 200-299)
    pub fn is_success(&self) -> bool {
        self.status() >= 200 && self.status() < 300
    }

    /// Check if response is client error (status 400-499)
    pub fn is_client_error(&self) -> bool {
        self.status() >= 400 && self.status() < 500
    }

    /// Check if response is server error (status 500-599)
    pub fn is_server_error(&self) -> bool {
        self.status() >= 500 && self.status() < 600
    }

    /// Check if response is any error (status >= 400)
    pub fn is_error(&self) -> bool {
        self.status() >= 400
    }
}

impl<'a> HttpRequestBuilder<'a> {
    fn new(client: &'a IpcHttpClient, method: &str, path: &str) -> Self {
        Self {
            client,
            method: method.to_string(),
            path: path.to_string(),
            body: None,
            timeout: None,
        }
    }

    /// Set request body as JSON
    pub fn json(mut self, body: &Value) -> Self {
        self.body = Some(body.clone());
        self
    }

    /// Set request body from serializable object
    pub fn json_body<T>(mut self, body: &T) -> AnyResult<Self>
    where
        T: serde::Serialize,
    {
        self.body = Some(serde_json::to_value(body)?);
        Ok(self)
    }

    /// Set request timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Execute the request and return enhanced response
    pub async fn send(self) -> AnyResult<HttpResponse> {
        let response = self
            .client
            .request_raw(&self.method, &self.path, self.body.as_ref())
            .await?;
        Ok(HttpResponse::new(response))
    }

    /// Execute request and directly get JSON result
    pub async fn json_result<T>(self) -> AnyResult<T>
    where
        T: DeserializeOwned,
    {
        let response = self.send().await?;
        response.json()
    }

    /// Execute request and get response text
    pub async fn text(self) -> AnyResult<String> {
        let response = self.send().await?;
        Ok(response.text().to_string())
    }

    /// Execute request and get JSON value
    pub async fn json_value(self) -> AnyResult<Value> {
        let response = self.send().await?;
        response.json_value()
    }
}

impl IpcHttpClient {
    /// Create a new IPC HTTP client with the given socket/named pipe path
    ///
    /// # Arguments
    /// * `path` - The path to the socket (Unix) or named pipe (Windows)
    pub fn new<P: AsRef<Path>>(path: P) -> AnyResult<Self> {
        let name = path.as_ref().to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Self {
            name,
            default_timeout: Duration::from_secs(30), // Default 30 seconds timeout
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
    pub fn get(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "GET", path)
    }

    /// HTTP-like POST request
    pub fn post(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "POST", path)
    }

    /// HTTP-like PUT request
    pub fn put(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "PUT", path)
    }

    /// HTTP-like DELETE request
    pub fn delete(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "DELETE", path)
    }

    /// HTTP-like PATCH request
    pub fn patch(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "PATCH", path)
    }

    /// HTTP-like HEAD request
    pub fn head(&self, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, "HEAD", path)
    }

    /// Generic request with custom method
    pub fn request(&self, method: &str, path: &str) -> HttpRequestBuilder {
        HttpRequestBuilder::new(self, method, path)
    }

    /// Low-level method: Send an HTTP request over the IPC connection
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `path` - Request path
    /// * `body` - Optional JSON body
    pub async fn request_raw(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> AnyResult<Response> {
        let stream = LocalSocketStream::connect(self.name.clone()).await?;
        http_request(stream, method, path, body).await
    }

    // =================== Backward Compatible Methods ===================

    /// Convenience method: Send GET request (backward compatible)
    pub async fn get_simple(&self, path: &str) -> AnyResult<Response> {
        self.request_raw("GET", path, None).await
    }

    /// Convenience method: Send POST request (backward compatible)
    pub async fn post_simple(&self, path: &str, body: &Value) -> AnyResult<Response> {
        self.request_raw("POST", path, Some(body)).await
    }

    /// Convenience method: Send PUT request (backward compatible)
    pub async fn put_simple(&self, path: &str, body: &Value) -> AnyResult<Response> {
        self.request_raw("PUT", path, Some(body)).await
    }

    /// Convenience method: Send DELETE request (backward compatible)
    pub async fn delete_simple(&self, path: &str) -> AnyResult<Response> {
        self.request_raw("DELETE", path, None).await
    }

    /// Convenience method: Send PATCH request (backward compatible)
    pub async fn patch_simple(&self, path: &str, body: &Value) -> AnyResult<Response> {
        self.request_raw("PATCH", path, Some(body)).await
    }
}
