//! HTTP-style IPC server implementation with routing and middleware support
//!
//! This module provides a high-level HTTP-style server for handling IPC requests
//! with a focus on ease of use, performance, and flexibility.

use crate::errors::{KodeBridgeError, Result};
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode, Uri};
use interprocess::local_socket::{
    GenericFilePath, ListenerOptions, Name, ToFsName, tokio::prelude::LocalSocketStream,
    traits::tokio::Listener,
};
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

/// Configuration for HTTP IPC server
#[derive(Debug, Clone)]
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
            max_connections: 100,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            max_request_size: 1024 * 1024, // 1MB
            enable_logging: true,
            shutdown_timeout: Duration::from_secs(5),
        }
    }
}

/// HTTP request context for handlers
#[derive(Debug)]
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
#[derive(Debug)]
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
struct Route {
    method: Method,
    path: String,
    handler: Arc<HandlerFn>,
}

/// Router for managing HTTP routes
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
        // Simple exact match for now
        // TODO: Implement path parameter matching (e.g., /users/{id})
        route_path == request_path
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

        Ok(Self {
            name,
            config: ServerConfig::default(),
            router: Arc::new(Router::new()),
            stats: Arc::new(RwLock::new(ServerStats::new())),
            connection_semaphore: Arc::new(Semaphore::new(ServerConfig::default().max_connections)),
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

        Ok(Self {
            name,
            config,
            router: Arc::new(Router::new()),
            stats: Arc::new(RwLock::new(ServerStats::new())),
            connection_semaphore,
            shutdown_tx: None,
        })
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
        let listener = ListenerOptions::new()
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
                                let config = self.config.clone();
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
                    } else {
                        break;
                    }
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
mod urlencoding {
    pub fn decode(input: &str) -> Result<std::borrow::Cow<str>, ()> {
        // Simple URL decoding implementation
        if input.contains('%') {
            let mut result = String::new();
            let mut chars = input.chars();

            while let Some(ch) = chars.next() {
                if ch == '%' {
                    let hex: String = chars.by_ref().take(2).collect();
                    if hex.len() == 2 {
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            result.push(byte as char);
                        } else {
                            return Err(());
                        }
                    } else {
                        return Err(());
                    }
                } else if ch == '+' {
                    result.push(' ');
                } else {
                    result.push(ch);
                }
            }
            Ok(std::borrow::Cow::Owned(result))
        } else {
            Ok(std::borrow::Cow::Borrowed(input))
        }
    }
}
