//! Streaming IPC server implementation for real-time data broadcasting
//!
//! This module provides a high-performance streaming server for broadcasting
//! real-time data to multiple clients with different streaming patterns.

use crate::errors::{KodeBridgeError, Result};
use bytes::Bytes;
#[cfg(unix)]
use interprocess::os::unix::local_socket::ListenerOptionsExt;
#[cfg(windows)]
use interprocess::os::windows::local_socket::ListenerOptionsExt;
#[cfg(windows)]
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use interprocess::{
    local_socket::{
        tokio::prelude::LocalSocketStream, traits::tokio::Listener, GenericFilePath,
        ListenerOptions, Name, ToFsName,
    },
    TryClone,
};
use parking_lot::RwLock;
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{broadcast, Semaphore},
    time::timeout,
};
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, error, info, warn};
#[cfg(windows)]
use widestring::U16CString;

/// Configuration for streaming IPC server
#[derive(Debug, Clone)]
pub struct StreamServerConfig {
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Buffer size for streaming data
    pub buffer_size: usize,
    /// Timeout for writing to clients
    pub write_timeout: Duration,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// Enable connection logging
    pub enable_logging: bool,
    /// Server shutdown timeout
    pub shutdown_timeout: Duration,
    /// Broadcast channel capacity
    pub broadcast_capacity: usize,
    /// Keep-alive interval for clients
    pub keepalive_interval: Duration,
}

impl Default for StreamServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 200,
            buffer_size: 65536,
            write_timeout: Duration::from_secs(10),
            max_message_size: 1024 * 1024, // 1MB
            enable_logging: true,
            shutdown_timeout: Duration::from_secs(5),
            broadcast_capacity: 1000,
            keepalive_interval: Duration::from_secs(30),
        }
    }
}

/// Message types for streaming
#[derive(Debug, Clone)]
pub enum StreamMessage {
    /// JSON data message
    Json(Value),
    /// Text message
    Text(String),
    /// Binary data
    Binary(Bytes),
    /// Keep-alive ping
    Ping,
    /// Server shutdown notification
    Close,
}

impl StreamMessage {
    /// Convert message to bytes for transmission
    pub fn to_bytes(&self) -> Bytes {
        match self {
            Self::Json(value) => {
                match serde_json::to_vec(value) {
                    Ok(bytes) => {
                        let mut output = Vec::with_capacity(bytes.len() + 1);
                        output.extend_from_slice(&bytes);
                        output.push(b'\n'); // Line delimiter
                        Bytes::from(output)
                    }
                    Err(_) => Bytes::from("{}\n"),
                }
            }
            Self::Text(text) => {
                let mut output = text.clone();
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                Bytes::from(output)
            }
            Self::Binary(bytes) => bytes.clone(),
            Self::Ping => Bytes::from("PING\n"),
            Self::Close => Bytes::from("CLOSE\n"),
        }
    }

    /// Create a JSON message
    pub fn json<T: serde::Serialize>(value: &T) -> Result<Self> {
        let json_value = serde_json::to_value(value)
            .map_err(|e| KodeBridgeError::json_serialize(format!("Failed to serialize: {}", e)))?;
        Ok(Self::Json(json_value))
    }

    /// Create a text message
    pub fn text<T: Into<String>>(text: T) -> Self {
        Self::Text(text.into())
    }

    /// Create a binary message
    pub fn binary<T: Into<Bytes>>(data: T) -> Self {
        Self::Binary(data.into())
    }
}

/// Client connection information for streaming
#[derive(Debug, Clone)]
pub struct StreamClient {
    /// Unique client ID
    pub client_id: u64,
    /// Connection establishment time
    pub connected_at: Instant,
    /// Last activity time
    pub last_activity: Instant,
    /// Client endpoint information
    pub endpoint: String,
    /// Number of messages sent to this client
    pub messages_sent: u64,
    /// Number of errors for this client
    pub error_count: u64,
}

impl StreamClient {
    fn new(client_id: u64, endpoint: String) -> Self {
        let now = Instant::now();
        Self {
            client_id,
            connected_at: now,
            last_activity: now,
            endpoint,
            messages_sent: 0,
            error_count: 0,
        }
    }
}

/// Statistics for the streaming server
#[derive(Debug, Clone)]
pub struct StreamServerStats {
    /// Total connections accepted
    pub total_connections: u64,
    /// Current active connections
    pub active_connections: u64,
    /// Total messages sent
    pub total_messages: u64,
    /// Total errors
    pub total_errors: u64,
    /// Messages per second (approximate)
    pub messages_per_second: f64,
    /// Server start time
    pub started_at: Instant,
    /// Last stats update time
    pub last_update: Instant,
}

impl StreamServerStats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            total_connections: 0,
            active_connections: 0,
            total_messages: 0,
            total_errors: 0,
            messages_per_second: 0.0,
            started_at: now,
            last_update: now,
        }
    }

    fn update_message_rate(&mut self, message_count: u64) {
        let now = Instant::now();
        let duration = now.duration_since(self.last_update).as_secs_f64();

        if duration > 0.0 {
            self.messages_per_second = message_count as f64 / duration;
        }

        self.total_messages += message_count;
        self.last_update = now;
    }
}

impl fmt::Display for StreamServerStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uptime = self.started_at.elapsed();
        write!(
            f,
            "Stream Server Stats: {} connections, {} active, {} messages ({:.1}/s), {} errors, uptime: {:?}",
            self.total_connections,
            self.active_connections,
            self.total_messages,
            self.messages_per_second,
            self.total_errors,
            uptime
        )
    }
}

/// Data source trait for streaming servers
pub trait StreamSource: Send + Sync {
    /// Get the next batch of messages to send
    fn next_messages(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>>;

    /// Check if the source has more data
    fn has_more(&self) -> bool;

    /// Initialize the source
    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Clean up the source
    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

/// Simple JSON data source that generates periodic messages
pub struct JsonDataSource {
    generator: Box<dyn Fn() -> Result<Value> + Send + Sync>,
    interval: Duration,
    last_generated: Instant,
}

impl JsonDataSource {
    /// Create a new JSON data source with a generator function
    pub fn new<F>(generator: F, interval: Duration) -> Self
    where
        F: Fn() -> Result<Value> + Send + Sync + 'static,
    {
        Self {
            generator: Box::new(generator),
            interval,
            last_generated: Instant::now(),
        }
    }
}

impl StreamSource for JsonDataSource {
    fn next_messages(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>> {
        Box::pin(async move {
            let now = Instant::now();
            if now.duration_since(self.last_generated) >= self.interval {
                self.last_generated = now;
                match (self.generator)() {
                    Ok(value) => Ok(vec![StreamMessage::Json(value)]),
                    Err(e) => Err(e),
                }
            } else {
                Ok(vec![])
            }
        })
    }

    fn has_more(&self) -> bool {
        true // Always has more for periodic generation
    }

    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }

    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }
}

/// Stream from an async iterator
pub struct IteratorSource<S> {
    stream: S,
}

impl<S> IteratorSource<S>
where
    S: Stream<Item = StreamMessage> + Send + Unpin,
{
    /// Create a new iterator source
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> StreamSource for IteratorSource<S>
where
    S: Stream<Item = StreamMessage> + Send + Sync + Unpin,
{
    fn next_messages(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>> {
        Box::pin(async move {
            match self.stream.next().await {
                Some(message) => Ok(vec![message]),
                None => Ok(vec![]),
            }
        })
    }

    fn has_more(&self) -> bool {
        true // Stream sources are considered to always have potential data
    }

    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }

    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }
}

/// High-level streaming IPC server
pub struct IpcStreamServer {
    name: Name<'static>,
    config: StreamServerConfig,
    listener_options: ListenerOptions<'static>,
    stats: Arc<RwLock<StreamServerStats>>,
    connection_semaphore: Arc<Semaphore>,
    clients: Arc<RwLock<HashMap<u64, StreamClient>>>,
    client_id_counter: Arc<AtomicU64>,
    broadcast_tx: Option<broadcast::Sender<StreamMessage>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl IpcStreamServer {
    /// Create a new streaming IPC server
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let name = path
            .as_ref()
            .to_fs_name::<GenericFilePath>()
            .map_err(|e| KodeBridgeError::configuration(format!("Invalid server path: {}", e)))?
            .into_owned();

        let config = StreamServerConfig::default();
        let connection_semaphore = Arc::new(Semaphore::new(config.max_connections));

        let listener_options = ListenerOptions::new();

        Ok(Self {
            name,
            config,
            listener_options,
            stats: Arc::new(RwLock::new(StreamServerStats::new())),
            connection_semaphore,
            clients: Arc::new(RwLock::new(HashMap::new())),
            client_id_counter: Arc::new(AtomicU64::new(1)),
            broadcast_tx: None,
            shutdown_tx: None,
        })
    }

    /// Create a new streaming IPC server with custom configuration
    pub fn with_config<P: AsRef<Path>>(path: P, config: StreamServerConfig) -> Result<Self> {
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
            stats: Arc::new(RwLock::new(StreamServerStats::new())),
            connection_semaphore,
            clients: Arc::new(RwLock::new(HashMap::new())),
            client_id_counter: Arc::new(AtomicU64::new(1)),
            broadcast_tx: None,
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
    /// server = server.with_listener_mode(0o660); // Only owner and group can read/write
    /// ```
    #[cfg(unix)]
    pub fn with_listener_mode(mut self, mode: u16) -> Self {
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

    /// Get server statistics
    pub fn stats(&self) -> StreamServerStats {
        self.stats.read().clone()
    }

    /// Get current clients
    pub fn clients(&self) -> Vec<StreamClient> {
        self.clients.read().values().cloned().collect()
    }

    /// Broadcast a message to all connected clients
    pub fn broadcast(&self, message: StreamMessage) -> Result<usize> {
        if let Some(ref tx) = self.broadcast_tx {
            match tx.send(message) {
                Ok(_) => Ok(tx.receiver_count()),
                Err(_) => Err(KodeBridgeError::connection("No active receivers")),
            }
        } else {
            Err(KodeBridgeError::connection("Server not started"))
        }
    }

    /// Start the server with a data source
    pub async fn serve_with_source<S>(&mut self, mut source: S) -> Result<()>
    where
        S: StreamSource + 'static,
    {
        // Initialize broadcast channel
        let (broadcast_tx, _) = broadcast::channel(self.config.broadcast_capacity);
        self.broadcast_tx = Some(broadcast_tx.clone());

        let listener_options = self.listener_options.try_clone()?;
        let listener = listener_options
            .name(self.name.clone())
            .create_tokio()
            .map_err(|e| KodeBridgeError::connection(format!("Failed to bind server: {}", e)))?;

        info!("🌊 Stream IPC Server listening on {:?}", self.name);

        // Initialize data source
        source.initialize().await?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Start data source task
        let source_stats = self.stats.clone();
        let source_broadcast_tx = broadcast_tx.clone();
        let (source_shutdown_tx, source_shutdown_rx) = tokio::sync::oneshot::channel();
        let source_shutdown = source_shutdown_rx;

        let source_task = tokio::spawn(async move {
            let mut source = source;
            let mut shutdown_rx = source_shutdown;

            loop {
                tokio::select! {
                    // Generate data
                    result = source.next_messages() => {
                        match result {
                            Ok(messages) => {
                                let message_count = messages.len() as u64;
                                for message in messages {
                                    if source_broadcast_tx.send(message).is_err() {
                                        debug!("No receivers for broadcast message");
                                    }
                                }
                                if message_count > 0 {
                                    source_stats.write().update_message_rate(message_count);
                                }
                            }
                            Err(e) => {
                                error!("Data source error: {}", e);
                                source_stats.write().total_errors += 1;
                            }
                        }

                        if !source.has_more() {
                            info!("Data source exhausted");
                            break;
                        }
                    }

                    // Handle shutdown
                    _ = &mut shutdown_rx => {
                        info!("Data source shutdown requested");
                        break;
                    }
                }

                // Small delay to prevent tight loops
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            // Cleanup source
            if let Err(e) = source.cleanup().await {
                error!("Error cleaning up data source: {}", e);
            }
        });

        // Main server loop
        loop {
            tokio::select! {
                // Accept new connections
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok(stream) => {
                            // Acquire connection permit
                            if let Ok(permit) = self.connection_semaphore.clone().try_acquire_owned() {
                                let client_id = self.client_id_counter.fetch_add(1, Ordering::SeqCst);

                                {
                                    let mut stats = self.stats.write();
                                    stats.total_connections += 1;
                                    stats.active_connections += 1;
                                }

                                let client = StreamClient::new(client_id, format!("client_{}", client_id));
                                self.clients.write().insert(client_id, client);

                                let config = self.config.clone();
                                let stats = self.stats.clone();
                                let clients = self.clients.clone();
                                let broadcast_rx = broadcast_tx.subscribe();

                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_stream_client(
                                        stream,
                                        client_id,
                                        broadcast_rx,
                                        config,
                                        stats.clone(),
                                        clients.clone(),
                                    ).await {
                                        error!("Stream client {} error: {}", client_id, e);
                                        stats.write().total_errors += 1;
                                    }

                                    // Remove client and update stats
                                    clients.write().remove(&client_id);
                                    {
                                        let mut stats = stats.write();
                                        stats.active_connections = stats.active_connections.saturating_sub(1);
                                    }

                                    drop(permit); // Release connection slot
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

                // Handle shutdown signal
                _ = &mut shutdown_rx => {
                    info!("Stream server shutdown requested");
                    break;
                }
            }
        }

        // Signal source task to stop
        let _ = source_shutdown_tx.send(());

        // Stop source task
        source_task.abort();

        // Send close message to all clients
        let _ = broadcast_tx.send(StreamMessage::Close);

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

        info!("Stream IPC Server stopped");
        Ok(())
    }

    /// Start the server with manual message broadcasting
    pub async fn serve(&mut self) -> Result<()> {
        // Create a dummy source that does nothing
        let dummy_source = JsonDataSource::new(
            || Ok(serde_json::json!({})),
            Duration::from_secs(3600), // Very long interval
        );

        self.serve_with_source(dummy_source).await
    }

    /// Shutdown the server gracefully
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Handle a single streaming client connection
    async fn handle_stream_client(
        mut stream: LocalSocketStream,
        client_id: u64,
        mut broadcast_rx: broadcast::Receiver<StreamMessage>,
        config: StreamServerConfig,
        stats: Arc<RwLock<StreamServerStats>>,
        clients: Arc<RwLock<HashMap<u64, StreamClient>>>,
    ) -> Result<()> {
        debug!("Handling stream client {}", client_id);

        let mut last_keepalive = Instant::now();

        loop {
            tokio::select! {
                // Receive broadcast messages
                msg_result = broadcast_rx.recv() => {
                    match msg_result {
                        Ok(message) => {
                            match message {
                                StreamMessage::Close => {
                                    debug!("Received close message for client {}", client_id);
                                    break;
                                }
                                _ => {
                                    // Send message to client
                                    let data = message.to_bytes();

                                    if data.len() > config.max_message_size {
                                        warn!("Message too large for client {}, skipping", client_id);
                                        continue;
                                    }

                                    match timeout(config.write_timeout, stream.write_all(&data)).await {
                                        Ok(Ok(())) => {
                                            if let Ok(()) = stream.flush().await {
                                                // Update client stats
                                                if let Some(client) = clients.write().get_mut(&client_id) {
                                                    client.messages_sent += 1;
                                                    client.last_activity = Instant::now();
                                                }
                                            }
                                        }
                                        Ok(Err(e)) => {
                                            error!("Failed to write to client {}: {}", client_id, e);
                                            if let Some(client) = clients.write().get_mut(&client_id) {
                                                client.error_count += 1;
                                            }
                                            stats.write().total_errors += 1;
                                            break;
                                        }
                                        Err(_) => {
                                            warn!("Write timeout for client {}", client_id);
                                            if let Some(client) = clients.write().get_mut(&client_id) {
                                                client.error_count += 1;
                                            }
                                            stats.write().total_errors += 1;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!("Client {} lagged behind, skipped {} messages", client_id, skipped);
                            if let Some(client) = clients.write().get_mut(&client_id) {
                                client.error_count += skipped;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            debug!("Broadcast channel closed for client {}", client_id);
                            break;
                        }
                    }
                }

                // Send periodic keepalive
                _ = tokio::time::sleep(config.keepalive_interval) => {
                    let now = Instant::now();
                    if now.duration_since(last_keepalive) >= config.keepalive_interval {
                        last_keepalive = now;

                        let ping_data = StreamMessage::Ping.to_bytes();
                        if let Err(e) = timeout(config.write_timeout, stream.write_all(&ping_data)).await {
                            warn!("Failed to send keepalive to client {}: {:?}", client_id, e);
                            break;
                        }

                        if let Err(e) = stream.flush().await {
                            warn!("Failed to flush keepalive to client {}: {}", client_id, e);
                            break;
                        }
                    }
                }
            }
        }

        debug!("Stream client {} finished", client_id);
        Ok(())
    }
}

impl fmt::Debug for IpcStreamServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpcStreamServer")
            .field("name", &self.name)
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish()
    }
}
