//! HTTP-style IPC server implementation with routing and middleware support
//!
//! This module provides a high-level HTTP-style server for handling IPC requests
//! with a focus on ease of use, performance, and flexibility.

use crate::errors::{KodeBridgeError, Result};
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, Uri};
use interprocess::local_socket::{
    tokio::prelude::LocalSocketStream, traits::tokio::Listener, GenericFilePath, ListenerOptions,
    Name, ToFsName,
};
#[cfg(unix)]
use interprocess::os::unix::local_socket::ListenerOptionsExt;
#[cfg(windows)]
use interprocess::os::windows::local_socket::ListenerOptionsExt;
#[cfg(windows)]
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use interprocess::TryClone;
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
    time::timeout,
};
use tracing::{debug, error, info, warn};
#[cfg(windows)]
use widestring::U16CString;

/// Configuration for HTTP IPC server
#[derive(Debug, Clone, Copy)]
pub struct ServerConfig {
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Timeout for reading requests
    pub read_timeout: Duration,
    /// Timeout for writing responses
    pub write_timeout: Duration,
    /// Maximum request body size in bytes
    pub max_request_size: usize,
    /// Enable request/response logging
    pub enable_logging: bool,
    /// Server shutdown timeout
    pub shutdown_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 200,                   // 增加最大连接数
            read_timeout: Duration::from_secs(15),  // 减少读超时
            write_timeout: Duration::from_secs(10), // 减少写超时
            max_request_size: 10 * 1024 * 1024,     // 增加到10MB支持大的PUT请求
            enable_logging: true,
            shutdown_timeout: Duration::from_secs(3), // 减少关闭超时
        }
    }
}

/// HTTP request context for handlers
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// HTTP method
    pub method: Method,
    /// Request URI
    pub uri: Uri,
    /// Request headers
    pub headers: HeaderMap,
    /// Request body as bytes
    pub body: Bytes,
    /// Client connection information
    pub client_info: ClientInfo,
    /// Request timestamp
    pub timestamp: Instant,
}

impl RequestContext {
    /// Parse request body as JSON
    pub fn json<T>(&self) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_slice(&self.body).map_err(|e| {
            KodeBridgeError::json_parse(format!("Failed to parse request JSON: {}", e))
        })
    }

    /// Get request body as UTF-8 string
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.to_vec()).map_err(|e| {
            KodeBridgeError::validation(format!("Invalid UTF-8 in request body: {}", e))
        })
    }

    /// Get query parameters from URI
    pub fn query_params(&self) -> HashMap<String, String> {
        if let Some(query) = self.uri.query() {
            query
                .split('&')
                .filter_map(|pair| {
                    let mut parts = pair.split('=');
                    match (parts.next(), parts.next()) {
                        (Some(key), Some(value)) => Some((
                            urlencoding::decode(key).ok()?.to_string(),
                            urlencoding::decode(value).ok()?.to_string(),
                        )),
                        (Some(key), None) => {
                            Some((urlencoding::decode(key).ok()?.to_string(), String::new()))
                        }
                        _ => None,
                    }
                })
                .collect()
        } else {
            HashMap::new()
        }
    }

    /// Get path parameters (when using route patterns)
    pub fn path_params(&self) -> HashMap<String, String> {
        // This would be populated by the router during route matching
        // For now, return empty map
        HashMap::new()
    }
}

/// Client connection information
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Connection ID for tracking
    pub connection_id: u64,
    /// Connection establishment time
    pub connected_at: Instant,
}

/// Response builder for HTTP responses
#[derive(Debug)]
pub struct ResponseBuilder {
    status: StatusCode,
    headers: HeaderMap,
    body: Option<Bytes>,
}

impl Default for ResponseBuilder {
    fn default() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
        }
    }
}

impl ResponseBuilder {
    /// Create a new response builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set response status code
    pub fn status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Add a header to the response
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: TryInto<http::header::HeaderName>,
        V: TryInto<http::header::HeaderValue>,
        K::Error: std::fmt::Debug,
        V::Error: std::fmt::Debug,
    {
        if let (Ok(k), Ok(v)) = (key.try_into(), value.try_into()) {
            self.headers.insert(k, v);
        }
        self
    }

    /// Set response body from bytes
    pub fn body<B: Into<Bytes>>(mut self, body: B) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set response body from JSON
    pub fn json<T: serde::Serialize>(self, value: &T) -> Result<Self> {
        let json_bytes = serde_json::to_vec(value).map_err(|e| {
            KodeBridgeError::json_serialize(format!("Failed to serialize JSON: {}", e))
        })?;

        Ok(self
            .header("content-type", "application/json")
            .body(json_bytes))
    }

    /// Set response body from text
    pub fn text<T: Into<String>>(self, text: T) -> Self {
        self.header("content-type", "text/plain; charset=utf-8")
            .body(text.into().into_bytes())
    }

    /// Build the final response
    pub fn build(self) -> HttpResponse {
        HttpResponse {
            status: self.status,
            headers: self.headers,
            body: self.body.unwrap_or_default(),
        }
    }
}

/// HTTP response representation
#[derive(Debug)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl HttpResponse {
    /// Create a new response builder
    pub fn builder() -> ResponseBuilder {
        ResponseBuilder::new()
    }

    /// Create a simple OK response
    pub fn ok() -> Self {
        ResponseBuilder::new().status(StatusCode::OK).build()
    }

    /// Create a JSON response
    pub fn json<T: serde::Serialize>(value: &T) -> Result<Self> {
        Ok(ResponseBuilder::new().json(value)?.build())
    }

    /// Create a text response
    pub fn text<T: Into<String>>(text: T) -> Self {
        ResponseBuilder::new().text(text).build()
    }

    /// Create an error response
    pub fn error(status: StatusCode, message: &str) -> Self {
        ResponseBuilder::new().status(status).text(message).build()
    }

    /// Create a 404 Not Found response
    pub fn not_found() -> Self {
        Self::error(StatusCode::NOT_FOUND, "Not Found")
    }

    /// Create a 500 Internal Server Error response
    pub fn internal_error() -> Self {
        Self::error(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
    }
}

/// Request handler function type
pub type HandlerFn = Box<
    dyn Fn(RequestContext) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send>>
        + Send
        + Sync,
>;

/// Route definition for the HTTP server
#[derive(Clone)]
struct Route {
    method: Method,
    path: String,
    handler: Arc<HandlerFn>,
}

/// Router for managing HTTP routes
#[derive(Clone)]
pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    /// Create a new router
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Add a GET route
    pub fn get<F, Fut>(self, path: &str, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
    {
        self.add_route(Method::GET, path, handler)
    }

    /// Add a POST route
    pub fn post<F, Fut>(self, path: &str, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
    {
        self.add_route(Method::POST, path, handler)
    }

    /// Add a PUT route
    pub fn put<F, Fut>(self, path: &str, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
    {
        self.add_route(Method::PUT, path, handler)
    }

    /// Add a DELETE route
    pub fn delete<F, Fut>(self, path: &str, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
    {
        self.add_route(Method::DELETE, path, handler)
    }

    /// Add a route with any HTTP method
    pub fn add_route<F, Fut>(mut self, method: Method, path: &str, handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HttpResponse>> + Send + 'static,
    {
        let handler_fn: HandlerFn = Box::new(move |ctx| Box::pin(handler(ctx)));

        self.routes.push(Route {
            method,
            path: path.to_string(),
            handler: Arc::new(handler_fn),
        });

        self
    }

    /// Find a matching route for the given method and path
    fn find_route(&self, method: &Method, path: &str) -> Option<&Route> {
        self.routes
            .iter()
            .find(|route| route.method == *method && self.path_matches(&route.path, path))
    }

    /// Check if a route path matches the request path
    fn path_matches(&self, route_path: &str, request_path: &str) -> bool {
        // Validate paths to prevent directory traversal attacks
        if !self.is_safe_path(request_path) {
            return false;
        }

        // Simple exact match for now
        // TODO: Implement path parameter matching (e.g., /users/{id})
        route_path == request_path
    }

    pub fn is_safe_path(&self, path: &str) -> bool {
        // Reject paths containing directory traversal patterns
        if path.contains("..") || path.contains("\\") {
            return false;
        }

        // Reject paths with null bytes or other control characters
        if path.contains('\0')
            || path
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return false;
        }

        // Ensure path starts with /
        if !path.starts_with('/') {
            return false;
        }

        // Reject excessively long paths
        if path.len() > 2048 {
            return false;
        }

        true
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Server statistics
#[derive(Debug, Clone)]
pub struct ServerStats {
    /// Total number of connections accepted
    pub total_connections: u64,
    /// Current active connections
    pub active_connections: u64,
    /// Total requests processed
    pub total_requests: u64,
    /// Total responses sent
    pub total_responses: u64,
    /// Total errors encountered
    pub total_errors: u64,
    /// Server start time
    pub started_at: Instant,
}

impl ServerStats {
    fn new() -> Self {
        Self {
            total_connections: 0,
            active_connections: 0,
            total_requests: 0,
            total_responses: 0,
            total_errors: 0,
            started_at: Instant::now(),
        }
    }
}

impl fmt::Display for ServerStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uptime = self.started_at.elapsed();
        write!(
            f,
            "Server Stats: {} total connections, {} active, {} requests, {} responses, {} errors, uptime: {:?}",
            self.total_connections,
            self.active_connections,
            self.total_requests,
            self.total_responses,
            self.total_errors,
            uptime
        )
    }
}

/// High-level HTTP IPC server
pub struct IpcHttpServer {
    name: Name<'static>,
    config: ServerConfig,
    listener_options: ListenerOptions<'static>,
    router: Arc<Router>,
    stats: Arc<RwLock<ServerStats>>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl IpcHttpServer {
    /// Create a new HTTP IPC server
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let name = path
            .as_ref()
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| KodeBridgeError::configuration(format!("Invalid server path: {}", e)))?
            .into_owned();

        let config = ServerConfig::default();
        let listener_options = ListenerOptions::new();

        Ok(Self {
            name,
            config,
            listener_options,
            router: Arc::new(Router::new()),
            stats: Arc::new(RwLock::new(ServerStats::new())),
            connection_semaphore: Arc::new(Semaphore::new(config.max_connections)),
            shutdown_tx: None,
        })
    }

    /// Create a new HTTP IPC server with custom configuration
    pub fn with_config<P: AsRef<Path>>(path: P, config: ServerConfig) -> Result<Self> {
        let name = path
            .as_ref()
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| KodeBridgeError::configuration(format!("Invalid server path: {}", e)))?
            .into_owned();

        let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));
        let listener_options = ListenerOptions::new();

        Ok(Self {
            name,
            config,
            listener_options,
            router: Arc::new(Router::new()),
            stats: Arc::new(RwLock::new(ServerStats::new())),
            connection_semaphore,
            shutdown_tx: None,
        })
    }

    pub fn with_listener_options(mut self, options: ListenerOptions<'static>) -> Self {
        self.listener_options = options;
        self
    }

    /// Sets the file mode for the Unix domain socket when creating the IPC listener.
    ///
    /// # Arguments
    /// * `mode` - A Unix file permission mask (e.g., `0o660` for owner/group read/write).
    ///
    /// # Example
    /// ```rust
    /// use kode_bridge::IpcHttpServer;
    ///
    /// let server = IpcHttpServer::new("/tmp/my.sock").unwrap().with_listener_mode(0o660); // Only owner and group can read/write
    /// ```
    #[cfg(unix)]
    pub fn with_listener_mode(mut self, mode: libc::mode_t) -> Self {
        self.listener_options = self.listener_options.mode(mode);
        self
    }

    /// Sets the security descriptor for Windows named pipe listener using a SDDL string.
    ///
    /// # Arguments
    /// * `sddl` - Security Descriptor Definition Language string (e.g., `"D:(A;;GA;;;WD)"` for Everyone access).
    ///
    /// # Panics
    /// Will panic if the SDDL string is invalid or cannot be parsed.
    ///
    /// # Example
    /// ```rust
    /// server = server.with_listener_security_descriptor("D:(A;;GA;;;WD)"); // Allow Everyone access
    /// ```
    ///
    /// # Reference
    /// See [Microsoft SDDL documentation](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format)
    #[cfg(windows)]
    pub fn with_listener_security_descriptor(mut self, sddl: &str) -> Self {
        let sddl = U16CString::from_str(sddl).expect("Invalid SDDL string");
        let sd = SecurityDescriptor::deserialize(&sddl).expect("Failed to parse SDDL");
        self.listener_options = self.listener_options.security_descriptor(sd);
        self
    }

    /// Set the router for handling requests
    pub fn router(mut self, router: Router) -> Self {
        self.router = Arc::new(router);
        self
    }

    /// Get server statistics
    pub fn stats(&self) -> ServerStats {
        self.stats.read().clone()
    }

    /// Start the server and listen for connections
    pub async fn serve(&mut self) -> Result<()> {
        let listener_options = self.listener_options.try_clone()?;
        let listener = listener_options
            .name(self.name.clone())
            .create_tokio()
            .map_err(|e| KodeBridgeError::connection(format!("Failed to bind server: {}", e)))?;

        info!("🚀 HTTP IPC Server listening on {:?}", self.name);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        loop {
            tokio::select! {
                // Accept new connections
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok(stream) => {
                            // Acquire connection permit
                            if let Ok(permit) = self.connection_semaphore.clone().try_acquire_owned() {
                                {
                                    let mut stats = self.stats.write();
                                    stats.total_connections += 1;
                                    stats.active_connections += 1;
                                }

                                let router = self.router.clone();
                                let config = self.config;
                                let stats = self.stats.clone();
                                let connection_id = {
                                    let stats = self.stats.read();
                                    stats.total_connections
                                };

                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_connection(
                                        stream,
                                        connection_id,
                                        router,
                                        config,
                                        stats.clone(),
                                    ).await {
                                        error!("Connection {} error: {}", connection_id, e);
                                        let mut stats = stats.write();
                                        stats.total_errors += 1;
                                    }

                                    // Connection finished
                                    {
                                        let mut stats = stats.write();
                                        stats.active_connections = stats.active_connections.saturating_sub(1);
                                    }

                                    drop(permit); // Release connection slot
                                });
                            } else {
                                warn!("Maximum connections reached, rejecting new connection");
                                // Connection will be dropped, closing it
                            }
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                        }
                    }
                }

                // Handle shutdown signal
                _ = &mut shutdown_rx => {
                    info!("Server shutdown requested");
                    break;
                }
            }
        }

        // Wait for active connections to finish
        let start = Instant::now();
        while self.stats.read().active_connections > 0
            && start.elapsed() < self.config.shutdown_timeout
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let remaining = self.stats.read().active_connections;
        if remaining > 0 {
            warn!("Shutting down with {} active connections", remaining);
        }

        info!("HTTP IPC Server stopped");
        Ok(())
    }

    /// Shutdown the server gracefully
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Handle a single client connection
    async fn handle_connection(
        mut stream: LocalSocketStream,
        connection_id: u64,
        router: Arc<Router>,
        config: ServerConfig,
        stats: Arc<RwLock<ServerStats>>,
    ) -> Result<()> {
        debug!("Handling connection {}", connection_id);

        let client_info = ClientInfo {
            connection_id,
            connected_at: Instant::now(),
        };

        loop {
            // Read HTTP request with timeout
            let request_data =
                match timeout(config.read_timeout, Self::read_request(&mut stream)).await {
                    Ok(Ok(Some(data))) => data,
                    Ok(Ok(None)) => {
                        debug!("Connection {} closed by client", connection_id);
                        break;
                    }
                    Ok(Err(e)) => {
                        error!(
                            "Failed to read request from connection {}: {}",
                            connection_id, e
                        );
                        return Err(e);
                    }
                    Err(_) => {
                        warn!("Read timeout on connection {}", connection_id);
                        return Err(KodeBridgeError::timeout_msg("Request read timeout"));
                    }
                };

            // Parse HTTP request
            let request_context =
                match Self::parse_request(request_data, &client_info, config.max_request_size) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        error!("Failed to parse request: {}", e);
                        let response = HttpResponse::error(StatusCode::BAD_REQUEST, "Bad Request");
                        Self::write_response(&mut stream, &response, &config).await?;
                        stats.write().total_errors += 1;
                        continue;
                    }
                };

            {
                let mut stats = stats.write();
                stats.total_requests += 1;
            }

            if config.enable_logging {
                info!(
                    "👤 {} {} {}",
                    request_context.method, request_context.uri, connection_id
                );
            }

            // Clone the method and URI for logging
            let method = request_context.method.clone();
            let uri = request_context.uri.clone();

            // Find and execute handler
            let response = if let Some(route) =
                router.find_route(&request_context.method, request_context.uri.path())
            {
                match timeout(config.write_timeout, (route.handler)(request_context)).await {
                    Ok(Ok(response)) => response,
                    Ok(Err(e)) => {
                        error!("Handler error: {}", e);
                        HttpResponse::internal_error()
                    }
                    Err(_) => {
                        warn!("Handler timeout");
                        HttpResponse::error(StatusCode::REQUEST_TIMEOUT, "Handler timeout")
                    }
                }
            } else {
                HttpResponse::not_found()
            };

            // Send response
            match Self::write_response(&mut stream, &response, &config).await {
                Ok(_) => {
                    stats.write().total_responses += 1;
                    if config.enable_logging {
                        info!("✅ {} {} - {}", method, uri, response.status);
                    }
                }
                Err(e) => {
                    error!("Failed to write response: {}", e);
                    stats.write().total_errors += 1;
                    return Err(e);
                }
            }

            // For HTTP/1.0 or if Connection: close, break the loop
            // For now, we'll keep the connection alive for multiple requests
        }

        debug!("Connection {} finished", connection_id);
        Ok(())
    }

    /// Read an HTTP request from the stream
    async fn read_request(stream: &mut LocalSocketStream) -> Result<Option<Vec<u8>>> {
        let mut buffer = vec![0u8; 8192];
        let mut total_data = Vec::new();

        loop {
            match stream.read(&mut buffer).await {
                Ok(0) => {
                    // Connection closed
                    if total_data.is_empty() {
                        return Ok(None);
                    }
                    break;
                }
                Ok(n) => {
                    total_data.extend_from_slice(&buffer[..n]);

                    // Check for end of HTTP headers (double CRLF)
                    if let Some(header_end) = Self::find_header_end(&total_data) {
                        // We have complete headers, now check if we need to read body
                        let headers_str = String::from_utf8_lossy(&total_data[..header_end]);

                        if let Some(content_length) = Self::extract_content_length(&headers_str) {
                            let body_start = header_end + 4; // Skip the \r\n\r\n
                            let total_expected = body_start + content_length;

                            // Read remaining body if needed
                            while total_data.len() < total_expected {
                                match stream.read(&mut buffer).await {
                                    Ok(0) => break, // Connection closed unexpectedly
                                    Ok(n) => total_data.extend_from_slice(&buffer[..n]),
                                    Err(e) => return Err(KodeBridgeError::Io(e)),
                                }
                            }
                        }
                        break;
                    }
                }
                Err(e) => return Err(KodeBridgeError::Io(e)),
            }
        }

        Ok(Some(total_data))
    }

    /// Find the end of HTTP headers (double CRLF)
    fn find_header_end(data: &[u8]) -> Option<usize> {
        data.windows(4).position(|window| window == b"\r\n\r\n")
    }

    /// Extract Content-Length from headers
    fn extract_content_length(headers: &str) -> Option<usize> {
        for line in headers.lines() {
            if let Some(value) = line
                .strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
            {
                if let Ok(length) = value.trim().parse::<usize>() {
                    return Some(length);
                }
            }
        }
        None
    }

    /// Parse raw HTTP request data into RequestContext
    fn parse_request(
        data: Vec<u8>,
        client_info: &ClientInfo,
        max_size: usize,
    ) -> Result<RequestContext> {
        if data.len() > max_size {
            return Err(KodeBridgeError::validation("Request too large"));
        }

        // Split headers and body
        let header_end = Self::find_header_end(&data).ok_or_else(|| {
            KodeBridgeError::validation("Invalid HTTP request: no header end found")
        })?;

        let headers_data = &data[..header_end];
        let body_data = if data.len() > header_end + 4 {
            &data[header_end + 4..]
        } else {
            &[]
        };

        // Parse request line and headers
        let headers_str = String::from_utf8_lossy(headers_data);
        let mut lines = headers_str.lines();

        let request_line = lines
            .next()
            .ok_or_else(|| KodeBridgeError::validation("Invalid HTTP request: no request line"))?;

        let mut parts = request_line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| KodeBridgeError::validation("Invalid HTTP request: no method"))?;
        let uri = parts
            .next()
            .ok_or_else(|| KodeBridgeError::validation("Invalid HTTP request: no URI"))?;

        // Parse method
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|e| KodeBridgeError::validation(format!("Invalid HTTP method: {}", e)))?;

        // Parse URI
        let uri = uri
            .parse::<Uri>()
            .map_err(|e| KodeBridgeError::validation(format!("Invalid URI: {}", e)))?;

        // Parse headers
        let mut headers = HeaderMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();

                if let (Ok(header_name), Ok(header_value)) = (
                    key.parse::<http::header::HeaderName>(),
                    value.parse::<http::header::HeaderValue>(),
                ) {
                    headers.insert(header_name, header_value);
                }
            }
        }

        Ok(RequestContext {
            method,
            uri,
            headers,
            body: Bytes::copy_from_slice(body_data),
            client_info: ClientInfo {
                connection_id: client_info.connection_id,
                connected_at: client_info.connected_at,
            },
            timestamp: Instant::now(),
        })
    }

    /// Write an HTTP response to the stream
    async fn write_response(
        stream: &mut LocalSocketStream,
        response: &HttpResponse,
        config: &ServerConfig,
    ) -> Result<()> {
        timeout(config.write_timeout, async {
            // Write status line
            let status_line = format!(
                "HTTP/1.1 {} {}\r\n",
                response.status.as_u16(),
                response.status.canonical_reason().unwrap_or("Unknown")
            );
            stream.write_all(status_line.as_bytes()).await?;

            // Write headers
            for (key, value) in &response.headers {
                let header_line = format!("{}: {}\r\n", key, value.to_str().unwrap_or(""));
                stream.write_all(header_line.as_bytes()).await?;
            }

            // Write Content-Length if not already present
            if !response.headers.contains_key("content-length") {
                let content_length = format!("Content-Length: {}\r\n", response.body.len());
                stream.write_all(content_length.as_bytes()).await?;
            }

            // End headers
            stream.write_all(b"\r\n").await?;

            // Write body
            if !response.body.is_empty() {
                stream.write_all(&response.body).await?;
            }

            stream.flush().await?;
            Ok::<(), KodeBridgeError>(())
        })
        .await
        .map_err(|_| KodeBridgeError::timeout_msg("Response write timeout"))?
    }
}

impl fmt::Debug for IpcHttpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpcHttpServer")
            .field("name", &self.name)
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish()
    }
}

// Helper for URL decoding
pub mod urlencoding {
    use std::borrow::Cow;

    #[derive(Debug)]
    pub struct DecodeError;

    impl std::fmt::Display for DecodeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Invalid URL encoding")
        }
    }

    impl std::error::Error for DecodeError {}

    pub fn decode(input: &str) -> Result<Cow<'_, str>, DecodeError> {
        // Safe URL decoding implementation with proper UTF-8 handling
        if !input.contains('%') && !input.contains('+') {
            return Ok(Cow::Borrowed(input));
        }

        let mut result = Vec::new();
        let bytes = input.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    if i + 2 < bytes.len() {
                        let hex_str =
                            std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| DecodeError)?;
                        let byte = u8::from_str_radix(hex_str, 16).map_err(|_| DecodeError)?;
                        result.push(byte);
                        i += 3;
                    } else {
                        return Err(DecodeError);
                    }
                }
                b'+' => {
                    result.push(b' ');
                    i += 1;
                }
                byte => {
                    result.push(byte);
                    i += 1;
                }
            }
        }

        // Validate UTF-8 and return
        let decoded_str = String::from_utf8(result).map_err(|_| DecodeError)?;
        Ok(Cow::Owned(decoded_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Method, StatusCode};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_router_can_be_cloned() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let router = Router::new()
            .get("/test", move |_ctx| {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(HttpResponse::text("Hello"))
                }
            })
            .post("/data", |_ctx| async {
                Ok(HttpResponse::json(&serde_json::json!({"status": "ok"})).unwrap())
            });

        // Clone the router
        let cloned_router = router.clone();

        // Verify both routers have the same routes
        assert_eq!(router.routes.len(), cloned_router.routes.len());
        assert_eq!(router.routes.len(), 2);

        // Test that both routers can find the same routes
        let get_route = router.find_route(&Method::GET, "/test");
        let cloned_get_route = cloned_router.find_route(&Method::GET, "/test");
        assert!(get_route.is_some());
        assert!(cloned_get_route.is_some());

        let post_route = router.find_route(&Method::POST, "/data");
        let cloned_post_route = cloned_router.find_route(&Method::POST, "/data");
        assert!(post_route.is_some());
        assert!(cloned_post_route.is_some());

        // Test that handlers work in both routers
        let ctx = RequestContext {
            method: Method::GET,
            uri: "/test".parse().unwrap(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            client_info: ClientInfo {
                connection_id: 1,
                connected_at: Instant::now(),
            },
            timestamp: Instant::now(),
        };

        // Execute handler from original router
        if let Some(route) = get_route {
            let response = (route.handler)(ctx.clone()).await.unwrap();
            assert_eq!(response.status, StatusCode::OK);
        }

        // Execute handler from cloned router
        if let Some(route) = cloned_get_route {
            let response = (route.handler)(ctx).await.unwrap();
            assert_eq!(response.status, StatusCode::OK);
        }

        // Verify the counter was incremented twice (once for each router)
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_router_clone_independence() {
        let router1 = Router::new().get("/original", |_ctx| async {
            Ok(HttpResponse::text("original"))
        });

        let mut router2 = router1.clone();

        // Add a route to the cloned router
        router2 = router2.post("/new", |_ctx| async { Ok(HttpResponse::text("new")) });

        // Original router should not have the new route
        assert_eq!(router1.routes.len(), 1);
        assert_eq!(router2.routes.len(), 2);

        // Verify routes
        assert!(router1.find_route(&Method::GET, "/original").is_some());
        assert!(router1.find_route(&Method::POST, "/new").is_none());

        assert!(router2.find_route(&Method::GET, "/original").is_some());
        assert!(router2.find_route(&Method::POST, "/new").is_some());
    }

    #[test]
    fn test_router_clone_with_multiple_methods() {
        let router = Router::new()
            .get("/users", |_ctx| async {
                Ok(HttpResponse::text("GET users"))
            })
            .post("/users", |_ctx| async {
                Ok(HttpResponse::text("POST users"))
            })
            .put("/users/123", |_ctx| async {
                Ok(HttpResponse::text("PUT user"))
            })
            .delete("/users/123", |_ctx| async {
                Ok(HttpResponse::text("DELETE user"))
            });

        let cloned_router = router.clone();

        // Both routers should have all methods
        assert_eq!(router.routes.len(), 4);
        assert_eq!(cloned_router.routes.len(), 4);

        // Test all HTTP methods on both routers
        let methods_and_paths = [
            (Method::GET, "/users"),
            (Method::POST, "/users"),
            (Method::PUT, "/users/123"),
            (Method::DELETE, "/users/123"),
        ];

        for (method, path) in &methods_and_paths {
            assert!(router.find_route(method, path).is_some());
            assert!(cloned_router.find_route(method, path).is_some());
        }
    }

    #[tokio::test]
    async fn test_cloned_router_handlers_work_independently() {
        let shared_state = Arc::new(AtomicU32::new(0));
        let state1 = shared_state.clone();
        let state2 = shared_state.clone();

        let router1 = Router::new().get("/increment", move |_ctx| {
            let state = state1.clone();
            async move {
                let value = state.fetch_add(1, Ordering::SeqCst);
                Ok(HttpResponse::text(format!("Router1: {}", value + 1)))
            }
        });

        let router2 = router1.clone().get("/decrement", move |_ctx| {
            let state = state2.clone();
            async move {
                let value = state.fetch_sub(1, Ordering::SeqCst);
                Ok(HttpResponse::text(format!("Router2: {}", value - 1)))
            }
        });

        let ctx = RequestContext {
            method: Method::GET,
            uri: "/increment".parse().unwrap(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            client_info: ClientInfo {
                connection_id: 1,
                connected_at: Instant::now(),
            },
            timestamp: Instant::now(),
        };

        // Test increment route exists in both routers
        let route1 = router1.find_route(&Method::GET, "/increment").unwrap();
        let route2 = router2.find_route(&Method::GET, "/increment").unwrap();

        let response1 = (route1.handler)(ctx.clone()).await.unwrap();
        let response2 = (route2.handler)(ctx).await.unwrap();

        // Both should work and increment the shared state
        assert_eq!(response1.status, StatusCode::OK);
        assert_eq!(response2.status, StatusCode::OK);
        assert_eq!(shared_state.load(Ordering::SeqCst), 2);

        // Test that router2 has the additional route
        assert!(router2.find_route(&Method::GET, "/decrement").is_some());
        assert!(router1.find_route(&Method::GET, "/decrement").is_none());
    }
}
