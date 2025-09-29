//! HTTP-style IPC server implementation with routing and middleware support
//!
//! This module provides a high-level HTTP-style server for handling IPC requests
//! with a focus on ease of use, performance, and flexibility.

use crate::errors::{KodeBridgeError, Result};
use bytes::{BufMut, Bytes, BytesMut};
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
use path_tree::PathTree;
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
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    sync::Semaphore,
    time::timeout,
};
use tracing::{debug, error, info, warn};
use url::Url;
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
    /// Maximum header size in bytes
    pub max_header_size: usize,
    /// Enable request/response logging
    pub enable_logging: bool,
    /// Server shutdown timeout
    pub shutdown_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 200,
            read_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(10),
            max_header_size: 4096,
            max_request_size: 10 * 1024 * 1024,
            enable_logging: true,
            shutdown_timeout: Duration::from_secs(3),
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
    /// Path parameters (when using route patterns)
    pub path_params: HashMap<String, String>,
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
        match Url::parse(&format!("http://localhost{}", self.uri)) {
            Ok(url) => url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect(),
            Err(_) => HashMap::new(),
        }
    }

    /// Get path parameters (when using route patterns)
    pub fn path_params(&self) -> HashMap<String, String> {
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

/// Router for managing HTTP routes
#[derive(Clone)]
pub struct Router {
    trees: HashMap<Method, PathTree<Arc<HandlerFn>>>,
}

impl Router {
    /// Create a new router
    pub fn new() -> Self {
        Self {
            trees: HashMap::new(),
        }
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
        let handler = Arc::new(handler_fn);
        let tree = self.trees.entry(method).or_insert_with(PathTree::new);
        let _ = tree.insert(path, handler);
        self
    }

    /// Find a matching handler and path parameters for the given method and path
    pub fn find_handler_and_params(
        &self,
        method: &Method,
        path: &str,
    ) -> Option<(Arc<HandlerFn>, HashMap<String, String>)> {
        let decoded = match urlencoding::decode(path) {
            Ok(cow) => cow.into_owned(),
            Err(_) => return None,
        };

        if !self.is_safe_path(&decoded) {
            return None;
        }

        if let Some(tree) = self.trees.get(method) {
            if let Some((handler, p)) = tree.find(&decoded) {
                let params: HashMap<String, String> = p
                    .params()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                return Some((handler.clone(), params));
            }
        }
        None
    }

    /// Check if a path is safe
    pub fn is_safe_path(&self, path: &str) -> bool {
        if path.contains("..") || path.contains("\\") {
            return false;
        }
        if path.contains('\0')
            || path
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return false;
        }
        if !path.starts_with('/') {
            return false;
        }
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
    pub total_connections: u64,
    pub active_connections: u64,
    pub total_requests: u64,
    pub total_responses: u64,
    pub total_errors: u64,
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

    #[cfg(unix)]
    pub fn with_listener_mode(mut self, mode: libc::mode_t) -> Self {
        self.listener_options = self.listener_options.mode(mode);
        self
    }

    #[cfg(windows)]
    pub fn with_listener_security_descriptor(mut self, sddl: &str) -> Self {
        let sddl = U16CString::from_str(sddl).expect("Invalid SDDL string");
        let sd = SecurityDescriptor::deserialize(&sddl).expect("Failed to parse SDDL");
        self.listener_options = self.listener_options.security_descriptor(sd);
        self
    }

    pub fn router(mut self, router: Router) -> Self {
        self.router = Arc::new(router);
        self
    }

    pub fn stats(&self) -> ServerStats {
        self.stats.read().clone()
    }

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
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok(stream) => {
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
                                    {
                                        let mut stats = stats.write();
                                        stats.active_connections = stats.active_connections.saturating_sub(1);
                                    }
                                    drop(permit);
                                });
                            } else {
                                warn!("Maximum connections reached, rejecting new connection");
                            }
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    info!("Server shutdown requested");
                    break;
                }
            }
        }

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

    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    async fn handle_connection(
        stream: LocalSocketStream,
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

        let mut stream =
            BufReader::with_capacity(config.max_header_size + config.max_request_size, stream);

        loop {
            let request_data = match timeout(
                config.read_timeout,
                Self::read_request(&mut stream, &config),
            )
            .await
            {
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

            let mut request_context =
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

            let method = request_context.method.clone();
            let uri = request_context.uri.clone();

            let response = if let Some((handler, params)) =
                router.find_handler_and_params(&request_context.method, request_context.uri.path())
            {
                request_context.path_params = params;
                match timeout(config.write_timeout, (handler)(request_context)).await {
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
        }

        debug!("Connection {} finished", connection_id);
        Ok(())
    }

    async fn read_request(
        stream: &mut BufReader<LocalSocketStream>,
        config: &ServerConfig,
    ) -> Result<Option<BytesMut>> {
        let mut buffer = BytesMut::with_capacity(config.max_header_size);

        loop {
            if buffer.len() > config.max_header_size {
                return Err(KodeBridgeError::validation(
                    "Header size exceeds maximum allowed",
                ));
            }

            let n = stream.read_buf(&mut buffer).await?;

            if n == 0 {
                if buffer.is_empty() {
                    return Ok(None);
                }
                break;
            }

            if let Some(header_end) = Self::find_header_end(&buffer) {
                let headers_str = String::from_utf8_lossy(&buffer[..header_end]);
                if let Some(content_length) = Self::extract_content_length(&headers_str) {
                    let body_start = header_end + 4;
                    let total_expected = body_start + content_length;

                    if total_expected > config.max_request_size {
                        return Err(KodeBridgeError::validation(
                            "Request size exceeds maximum allowed",
                        ));
                    }

                    while buffer.len() < total_expected {
                        let n = stream.read_buf(&mut buffer).await?;
                        if n == 0 {
                            break;
                        }
                    }
                }
                break;
            }
        }

        Ok(Some(buffer))
    }

    fn find_header_end(data: &[u8]) -> Option<usize> {
        data.windows(4).position(|window| window == b"\r\n\r\n")
    }

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

    fn parse_request(
        data: BytesMut,
        client_info: &ClientInfo,
        max_size: usize,
    ) -> Result<RequestContext> {
        if data.len() > max_size {
            return Err(KodeBridgeError::validation("Request too large"));
        }

        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        let res = req.parse(&data).map_err(|e| {
            KodeBridgeError::validation(format!("Failed to parse HTTP request: {}", e))
        })?;

        if res.is_partial() {
            return Err(KodeBridgeError::validation("Incomplete HTTP request"));
        }

        let method = req
            .method
            .ok_or_else(|| KodeBridgeError::validation("Missing HTTP method"))?;
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|e| KodeBridgeError::validation(format!("Invalid HTTP method: {}", e)))?;

        let path = req
            .path
            .ok_or_else(|| KodeBridgeError::validation("Missing HTTP path"))?;
        let uri = path
            .parse::<Uri>()
            .map_err(|e| KodeBridgeError::validation(format!("Invalid URI: {}", e)))?;

        let mut header_map = HeaderMap::new();
        for header in req.headers {
            if let (Ok(name), Ok(value)) = (
                header.name.parse::<http::header::HeaderName>(),
                std::str::from_utf8(header.value)
                    .map_err(|_| KodeBridgeError::validation("Invalid header value"))?
                    .parse::<http::header::HeaderValue>(),
            ) {
                header_map.insert(name, value);
            }
        }

        let body_start = res.unwrap();
        let body = if data.len() > body_start {
            Bytes::copy_from_slice(&data[body_start..])
        } else {
            Bytes::new()
        };

        Ok(RequestContext {
            method,
            uri,
            headers: header_map,
            body,
            client_info: ClientInfo {
                connection_id: client_info.connection_id,
                connected_at: client_info.connected_at,
            },
            timestamp: Instant::now(),
            path_params: HashMap::new(),
        })
    }

    async fn write_response(
        stream: &mut BufReader<LocalSocketStream>,
        response: &HttpResponse,
        config: &ServerConfig,
    ) -> Result<()> {
        timeout(config.write_timeout, async {
            // Use BytesMut for efficient buffer building
            let mut buf = BytesMut::with_capacity(4096);

            // Status line
            buf.put(
                format!(
                    "HTTP/1.1 {} {}\r\n",
                    response.status.as_u16(),
                    response.status.canonical_reason().unwrap_or("Unknown")
                )
                .as_bytes(),
            );

            // Headers
            let mut has_content_length = false;
            for (key, value) in response.headers.iter() {
                buf.put(format!("{}: {}\r\n", key, value.to_str().unwrap_or("")).as_bytes());
                if key.as_str().eq_ignore_ascii_case("content-length") {
                    has_content_length = true;
                }
            }

            // Content-Length header if not present
            if !has_content_length {
                buf.put(format!("Content-Length: {}\r\n", response.body.len()).as_bytes());
            }

            // End of headers
            buf.put_slice(b"\r\n");

            // Body
            buf.put(response.body.as_ref());

            // Write buffer
            stream.write_all(&buf).await?;
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

        let cloned_router = router.clone();

        assert_eq!(router.trees.len(), cloned_router.trees.len());
        assert_eq!(router.trees.len(), 2);

        let get_route = router.find_handler_and_params(&Method::GET, "/test");
        let cloned_get_route = cloned_router.find_handler_and_params(&Method::GET, "/test");
        assert!(get_route.is_some());
        assert!(cloned_get_route.is_some());

        let post_route = router.find_handler_and_params(&Method::POST, "/data");
        let cloned_post_route = cloned_router.find_handler_and_params(&Method::POST, "/data");
        assert!(post_route.is_some());
        assert!(cloned_post_route.is_some());

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
            path_params: HashMap::new(),
        };

        if let Some((handler, _)) = get_route {
            let response = (handler)(ctx.clone()).await.unwrap();
            assert_eq!(response.status, StatusCode::OK);
        }

        if let Some((handler, _)) = cloned_get_route {
            let response = (handler)(ctx).await.unwrap();
            assert_eq!(response.status, StatusCode::OK);
        }

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_router_clone_independence() {
        let router1 = Router::new().get("/original", |_ctx| async {
            Ok(HttpResponse::text("original"))
        });

        let mut router2 = router1.clone();
        router2 = router2.post("/new", |_ctx| async { Ok(HttpResponse::text("new")) });

        assert_eq!(router1.trees.len(), 1);
        assert_eq!(router2.trees.len(), 2);

        assert!(router1
            .find_handler_and_params(&Method::GET, "/original")
            .is_some());
        assert!(router1
            .find_handler_and_params(&Method::POST, "/new")
            .is_none());
        assert!(router2
            .find_handler_and_params(&Method::GET, "/original")
            .is_some());
        assert!(router2
            .find_handler_and_params(&Method::POST, "/new")
            .is_some());
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

        assert_eq!(router.trees.len(), 4);
        assert_eq!(cloned_router.trees.len(), 4);

        let methods_and_paths = [
            (Method::GET, "/users"),
            (Method::POST, "/users"),
            (Method::PUT, "/users/123"),
            (Method::DELETE, "/users/123"),
        ];

        for (method, path) in &methods_and_paths {
            assert!(router.find_handler_and_params(method, path).is_some());
            assert!(cloned_router
                .find_handler_and_params(method, path)
                .is_some());
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
            path_params: HashMap::new(),
        };

        let (handler1, _) = router1
            .find_handler_and_params(&Method::GET, "/increment")
            .unwrap();
        let (handler2, _) = router2
            .find_handler_and_params(&Method::GET, "/increment")
            .unwrap();

        let response1 = (handler1)(ctx.clone()).await.unwrap();
        let response2 = (handler2)(ctx).await.unwrap();

        assert_eq!(response1.status, StatusCode::OK);
        assert_eq!(response2.status, StatusCode::OK);
        assert_eq!(shared_state.load(Ordering::SeqCst), 2);

        assert!(router2
            .find_handler_and_params(&Method::GET, "/decrement")
            .is_some());
        assert!(router1
            .find_handler_and_params(&Method::GET, "/decrement")
            .is_none());
    }

    #[tokio::test]
    async fn test_path_params() {
        let router = Router::new().get("/users/:id", |ctx| async move {
            let params = ctx.path_params();
            let id = params.get("id").cloned().unwrap_or_default();
            Ok(HttpResponse::text(format!("User ID: {}", id)))
        });

        let ctx = RequestContext {
            method: Method::GET,
            uri: "/users/123".parse().unwrap(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            client_info: ClientInfo {
                connection_id: 1,
                connected_at: Instant::now(),
            },
            timestamp: Instant::now(),
            path_params: HashMap::new(),
        };

        let (handler, params) = router
            .find_handler_and_params(&Method::GET, "/users/123")
            .unwrap();
        let mut ctx = ctx;
        ctx.path_params = params;

        let response = (handler)(ctx).await.unwrap();
        assert_eq!(response.body, b"User ID: 123");
    }
}
