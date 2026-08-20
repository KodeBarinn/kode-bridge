#![allow(clippy::panic)]

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use futures::future::join_all;
use http::Method;
use kode_bridge::ipc_http_client::{ClientConfig, IpcHttpClient};
use kode_bridge::ipc_http_server::{HttpResponse, IpcHttpServer, Router, ServerConfig};
use kode_bridge::ipc_stream_server::{IpcStreamServer, StreamServerConfig, StreamSource};
use kode_bridge::metrics::MetricsCollector;
use kode_bridge::pool::PoolConfig;
use kode_bridge::retry::{JitterStrategy, RetryConfig, RetryExecutor};
use kode_bridge::{KodeBridgeError, Result, StreamMessage};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_POOL_SIZE: usize = 40;
static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct HttpBenchContext {
    runtime: Runtime,
    endpoint: PathBuf,
    server_task: Option<JoinHandle<kode_bridge::Result<()>>>,
    client: IpcHttpClient,
    router: Router,
}

impl HttpBenchContext {
    fn new() -> Self {
        let runtime = Runtime::new().unwrap_or_else(|error| panic!("failed to create benchmark runtime: {error}"));
        let endpoint = unique_endpoint("http");
        let router = benchmark_router();
        let server = IpcHttpServer::with_config(&endpoint, server_config())
            .unwrap_or_else(|error| panic!("failed to configure benchmark server: {error}"))
            .router(router.clone());
        let server_task = runtime.spawn(async move {
            let mut server = server;
            server.serve().await
        });
        runtime.block_on(wait_until_ready(&endpoint, &server_task));
        let client = build_client(&endpoint, true);
        Self {
            runtime,
            endpoint,
            server_task: Some(server_task),
            client,
            router,
        }
    }
}

impl Drop for HttpBenchContext {
    fn drop(&mut self) {
        self.client.close();
        if let Some(task) = self.server_task.take() {
            task.abort();
            let result = self.runtime.block_on(task);
            assert!(result.is_err_and(|error| error.is_cancelled()));
        }
    }
}

fn unique_endpoint(label: &str) -> PathBuf {
    let sequence = ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    #[cfg(unix)]
    {
        PathBuf::from(format!(
            "/tmp/kb-hot-path-{}-{sequence}-{label}.sock",
            std::process::id()
        ))
    }

    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\kb-hot-path-{}-{sequence}-{label}",
            std::process::id()
        ))
    }
}

const fn server_config() -> ServerConfig {
    ServerConfig {
        max_connections: MAX_POOL_SIZE + 8,
        read_timeout: REQUEST_TIMEOUT,
        write_timeout: REQUEST_TIMEOUT,
        max_request_size: 2 * 1024 * 1024,
        max_header_size: 16 * 1024,
        enable_logging: false,
        max_requests_per_connection: usize::MAX,
        shutdown_timeout: Duration::from_secs(1),
    }
}

const fn client_config(enable_pooling: bool) -> ClientConfig {
    ClientConfig {
        default_timeout: REQUEST_TIMEOUT,
        pool_config: PoolConfig {
            max_size: MAX_POOL_SIZE,
            min_idle: 0,
            max_idle_time_ms: 30_000,
            connection_timeout_ms: 1_000,
            retry_delay_ms: 5,
            max_retries: 1,
            max_concurrent_requests: MAX_POOL_SIZE,
            max_requests_per_second: None,
        },
        enable_pooling,
        max_retries: 1,
        retry_delay: Duration::from_millis(5),
        max_concurrent_requests: MAX_POOL_SIZE,
        max_requests_per_second: None,
    }
}

fn build_client(path: &Path, enable_pooling: bool) -> IpcHttpClient {
    IpcHttpClient::with_config(path, client_config(enable_pooling))
        .unwrap_or_else(|error| panic!("failed to create benchmark client: {error}"))
}

fn benchmark_router() -> Router {
    Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/literal", |_ctx| async { Ok(HttpResponse::text("literal")) })
        .get("/items/:id", |ctx| async move {
            Ok(HttpResponse::text(ctx.path_params["id"].clone()))
        })
        .get("/query", |ctx| async move {
            Ok(HttpResponse::text(
                ctx.query_params().get("page").cloned().unwrap_or_default(),
            ))
        })
        .get("/response", |_ctx| async {
            Ok(HttpResponse::builder()
                .header("content-type", "application/json")
                .body(Bytes::from_static(b"{\"status\":\"ok\",\"items\":[1,2,3]}"))
                .build())
        })
        .get("/fragmented", |_ctx| async { Ok(HttpResponse::text("fragmented")) })
}

async fn wait_until_ready(endpoint: &Path, server_task: &JoinHandle<kode_bridge::Result<()>>) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        assert!(!server_task.is_finished(), "benchmark server exited before readiness");
        let client = build_client(endpoint, false);
        if let Ok(Ok(response)) = timeout(Duration::from_millis(250), client.get("/ready").send()).await {
            if response.is_success() {
                return;
            }
        }
        assert!(Instant::now() < deadline, "benchmark server readiness timed out");
        tokio::task::yield_now().await;
    }
}

#[cfg(unix)]
type RawStream = tokio::net::UnixStream;
#[cfg(windows)]
type RawStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(unix)]
async fn raw_connect(path: &Path) -> std::io::Result<RawStream> {
    RawStream::connect(path).await
}

#[cfg(windows)]
async fn raw_connect(path: &Path) -> std::io::Result<RawStream> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(path)
}

async fn fragmented_request(endpoint: &Path) {
    let mut stream = raw_connect(endpoint)
        .await
        .unwrap_or_else(|error| panic!("failed to create fragmented-header client: {error}"));
    stream
        .write_all(b"GET /fragmented HTTP/1.1\r\nHost: benchmark")
        .await
        .unwrap_or_else(|error| panic!("failed to write initial header fragment: {error}"));
    stream
        .write_all(b"\r\n\r\n")
        .await
        .unwrap_or_else(|error| panic!("failed to write header terminator: {error}"));
    stream
        .flush()
        .await
        .unwrap_or_else(|error| panic!("failed to flush fragmented request: {error}"));
    let mut response = [0u8; 128];
    let bytes_read = timeout(REQUEST_TIMEOUT, stream.read(&mut response))
        .await
        .unwrap_or_else(|_| panic!("fragmented-header response timed out"))
        .unwrap_or_else(|error| panic!("failed to read fragmented-header response: {error}"));
    assert!(response[..bytes_read].starts_with(b"HTTP/1.1 200"));
}

const STREAM_WARMUP: &str = "warmup";
const STREAM_PAYLOAD: &str = "payload";

struct ControlledSource {
    messages: Arc<Mutex<mpsc::Receiver<StreamMessage>>>,
    initialized: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl StreamSource for ControlledSource {
    fn next_messages(&mut self) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>> {
        Box::pin(async move {
            let mut messages = self.messages.lock().await;
            Ok(messages.recv().await.into_iter().collect())
        })
    }

    fn has_more(&self) -> bool {
        true
    }

    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            if let Some(ready) = self
                .initialized
                .lock()
                .expect("source readiness lock poisoned")
                .take()
            {
                let _ = ready.send(());
            }
            Ok(())
        })
    }

    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

enum StreamEvent {
    Warmed,
    ReceivedPayload,
}

struct StreamBroadcastContext {
    sender: mpsc::Sender<StreamMessage>,
    events: mpsc::Receiver<StreamEvent>,
    client_tasks: Vec<JoinHandle<()>>,
    server_task: JoinHandle<kode_bridge::Result<()>>,
}

impl StreamBroadcastContext {
    async fn start(endpoint: PathBuf, clients: usize) -> Self {
        let (message_tx, message_rx) = mpsc::channel(128);
        let (source_ready_tx, source_ready_rx) = oneshot::channel();
        let source = ControlledSource {
            messages: Arc::new(Mutex::new(message_rx)),
            initialized: Arc::new(std::sync::Mutex::new(Some(source_ready_tx))),
        };
        let mut server = IpcStreamServer::with_config(
            &endpoint,
            StreamServerConfig {
                max_connections: clients + 4,
                buffer_size: 8192,
                write_timeout: REQUEST_TIMEOUT,
                max_message_size: 64 * 1024,
                enable_logging: false,
                shutdown_timeout: Duration::from_secs(1),
                broadcast_capacity: 128,
                keepalive_interval: Duration::from_secs(60),
            },
        )
        .unwrap_or_else(|error| panic!("failed to configure streaming benchmark server: {error}"));
        let server_task = tokio::spawn(async move { server.serve_with_source(source).await });
        timeout(STARTUP_TIMEOUT, source_ready_rx)
            .await
            .unwrap_or_else(|_| panic!("streaming benchmark server readiness timed out"))
            .unwrap_or_else(|_| panic!("streaming benchmark server dropped readiness signal"));

        let (event_tx, event_rx) = mpsc::channel(clients * 4);
        let mut client_tasks = Vec::with_capacity(clients);
        for _ in 0..clients {
            let endpoint = endpoint.clone();
            let event_tx = event_tx.clone();
            client_tasks.push(tokio::spawn(async move {
                let stream = raw_connect(&endpoint)
                    .await
                    .unwrap_or_else(|error| panic!("failed to connect streaming benchmark client: {error}"));
                let mut reader = BufReader::new(stream);
                let mut line = Vec::with_capacity(64);
                let mut warmed = false;
                loop {
                    line.clear();
                    let bytes = reader
                        .read_until(b'\n', &mut line)
                        .await
                        .unwrap_or_else(|error| panic!("failed to read streaming benchmark message: {error}"));
                    if bytes == 0 {
                        break;
                    }
                    if line.as_slice() == b"warmup\n" && !warmed {
                        warmed = true;
                        let _ = event_tx.send(StreamEvent::Warmed).await;
                    } else if line.as_slice() == b"payload\n" {
                        let _ = event_tx.send(StreamEvent::ReceivedPayload).await;
                    }
                }
            }));
        }
        drop(event_tx);

        let mut context = Self {
            sender: message_tx,
            events: event_rx,
            client_tasks,
            server_task,
        };
        context.wait_until_clients_are_subscribed(clients).await;
        context
    }

    async fn wait_until_clients_are_subscribed(&mut self, clients: usize) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut warmed = 0usize;
        while warmed < clients {
            self.sender
                .send(StreamMessage::text(STREAM_WARMUP))
                .await
                .unwrap_or_else(|error| panic!("failed to publish stream warmup: {error}"));
            while let Ok(event) = self.events.try_recv() {
                if matches!(event, StreamEvent::Warmed) {
                    warmed += 1;
                }
            }
            assert!(
                Instant::now() < deadline,
                "streaming benchmark clients did not subscribe in time"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn broadcast_once(&mut self, clients: usize) {
        self.sender
            .send(StreamMessage::text(STREAM_PAYLOAD))
            .await
            .unwrap_or_else(|error| panic!("failed to publish benchmark payload: {error}"));
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut received = 0usize;
        while received < clients {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = timeout(remaining, self.events.recv())
                .await
                .unwrap_or_else(|_| panic!("streaming benchmark payload timed out"))
                .unwrap_or_else(|| panic!("streaming benchmark event channel closed"));
            if matches!(event, StreamEvent::ReceivedPayload) {
                received += 1;
            }
        }
    }
}

impl Drop for StreamBroadcastContext {
    fn drop(&mut self) {
        for task in &self.client_tasks {
            task.abort();
        }
        self.server_task.abort();
    }
}

fn bench_router_and_response(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_router");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(40);
    for (name, path) in [
        ("literal", "/literal"),
        ("param", "/items/42"),
        ("query", "/query?page=42&sort=name"),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                assert!(context
                    .router
                    .find_handler_and_params(&Method::GET, path)
                    .is_some());
            });
        });
    }
    group.finish();

    let mut response_group = c.benchmark_group("hot_path_response_parse");
    response_group.measurement_time(Duration::from_secs(6));
    response_group.sample_size(40);
    response_group.bench_function("client_response", |bencher| {
        bencher.to_async(&context.runtime).iter(|| async {
            let response = context
                .client
                .get("/response")
                .send()
                .await
                .unwrap_or_else(|error| panic!("response benchmark failed: {error}"));
            assert_eq!(response.json_value().expect("response must be JSON")["status"], "ok");
        });
    });
    response_group.finish();
}

fn bench_metrics(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_metrics");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(30);
    for clients in [1usize, 32, 128] {
        group.throughput(Throughput::Elements(clients as u64));
        group.bench_with_input(
            BenchmarkId::new("request_tracker", clients),
            &clients,
            |bencher, &clients| {
                bencher.to_async(&context.runtime).iter_batched(
                    || Arc::new(MetricsCollector::new()),
                    |metrics| async move {
                        let trackers = (0..clients).map(|_| {
                            let metrics = Arc::clone(&metrics);
                            async move { metrics.request_start("GET").success(200) }
                        });
                        join_all(trackers).await;
                        assert_eq!(metrics.snapshot().successful_requests, clients as u64);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_pool_preheat(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_pool_preheat");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(20);
    for connections in [8usize, 32] {
        group.throughput(Throughput::Elements(connections as u64));
        group.bench_with_input(
            BenchmarkId::new("connections", connections),
            &connections,
            |bencher, &connections| {
                let endpoint = context.endpoint.clone();
                bencher.to_async(&context.runtime).iter_batched(
                    || build_client(&endpoint, true),
                    |client| async move {
                        client.preheat_for_puts(connections).await;
                        let stats = client.pool_stats().expect("pooling is enabled");
                        assert_eq!(stats.total_connections, connections);
                        client.close();
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_fragmented_headers(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_codec_fragmented_headers");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(30);
    group.bench_function("two_fragments", |bencher| {
        bencher
            .to_async(&context.runtime)
            .iter(|| fragmented_request(&context.endpoint));
    });
    group.finish();
}

fn bench_stream_broadcast(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_stream_broadcast");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(20);
    for clients in [1usize, 32, 64] {
        group.throughput(Throughput::Elements(clients as u64));
        group.bench_with_input(
            BenchmarkId::new("broadcast_write_flush", clients),
            &clients,
            |bencher, &clients| {
                let endpoint = unique_endpoint("stream");
                let broadcast = Arc::new(Mutex::new(
                    context
                        .runtime
                        .block_on(StreamBroadcastContext::start(endpoint, clients)),
                ));
                bencher.to_async(&context.runtime).iter(|| {
                    let broadcast = Arc::clone(&broadcast);
                    async move {
                        broadcast.lock().await.broadcast_once(clients).await;
                    }
                });
            },
        );
    }
    group.finish();
}

fn bench_retry_first_success(c: &mut Criterion, context: &HttpBenchContext) {
    let mut group = c.benchmark_group("hot_path_retry");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(50);
    group.bench_function("first_success_no_jitter", |bencher| {
        bencher.to_async(&context.runtime).iter(|| async {
            let executor = RetryExecutor::new(
                RetryConfig::new()
                    .max_attempts(3)
                    .base_delay(Duration::from_secs(1))
                    .jitter(JitterStrategy::None),
            );
            let result = executor
                .execute(|| async { Ok::<_, KodeBridgeError>(()) })
                .await
                .unwrap_or_else(|error| panic!("first-success retry benchmark failed: {error}"));
            assert_eq!(result, ());
        });
    });
    group.finish();
}

fn benchmark_hot_path(c: &mut Criterion) {
    let context = HttpBenchContext::new();
    bench_router_and_response(c, &context);
    bench_metrics(c, &context);
    bench_pool_preheat(c, &context);
    bench_fragmented_headers(c, &context);
    bench_stream_broadcast(c, &context);
    bench_retry_first_success(c, &context);
}

criterion_group!(benches, benchmark_hot_path);
criterion_main!(benches);
