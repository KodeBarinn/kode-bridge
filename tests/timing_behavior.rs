#![cfg(all(feature = "client", feature = "server"))]

use bytes::Bytes;
use kode_bridge::ipc_http_client::{ClientConfig, IpcHttpClient};
use kode_bridge::ipc_http_server::{HttpResponse, IpcHttpServer, Router, ServerConfig};
use kode_bridge::pool::PoolConfig;
use kode_bridge::retry::{JitterStrategy, RetryConfig, RetryExecutor};
use kode_bridge::{KodeBridgeError, Result};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};

type TestResult<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn unique_endpoint(label: &str) -> PathBuf {
    let sequence = ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    #[cfg(unix)]
    {
        PathBuf::from(format!("/tmp/kb-timing-{}-{sequence}-{label}.sock", std::process::id()))
    }

    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\kb-timing-{}-{sequence}-{label}", std::process::id()))
    }
}

const fn client_config() -> ClientConfig {
    ClientConfig {
        default_timeout: REQUEST_TIMEOUT,
        pool_config: PoolConfig {
            max_size: 8,
            min_idle: 0,
            max_idle_time_ms: 30_000,
            connection_timeout_ms: 1_000,
            retry_delay_ms: 5,
            max_retries: 1,
            max_concurrent_requests: 8,
            max_requests_per_second: None,
        },
        enable_pooling: false,
        max_retries: 1,
        retry_delay: Duration::from_millis(5),
        max_concurrent_requests: 8,
        max_requests_per_second: None,
    }
}

const fn server_config(max_connections: usize) -> ServerConfig {
    ServerConfig {
        max_connections,
        read_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(2),
        max_request_size: 2 * 1024 * 1024,
        max_header_size: 16 * 1024,
        enable_logging: false,
        max_requests_per_connection: 1,
        shutdown_timeout: Duration::from_secs(1),
    }
}

struct HttpServerGuard {
    endpoint: PathBuf,
    task: Option<JoinHandle<Result<()>>>,
}

impl HttpServerGuard {
    async fn start(endpoint: PathBuf, config: ServerConfig, router: Router) -> TestResult<Self> {
        let mut server = IpcHttpServer::with_config(&endpoint, config)?.router(router);
        let task = tokio::spawn(async move { server.serve().await });
        let mut guard = Self {
            endpoint,
            task: Some(task),
        };
        guard.wait_until_ready().await?;
        Ok(guard)
    }

    async fn wait_until_ready(&mut self) -> TestResult {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if self.task.as_ref().is_some_and(JoinHandle::is_finished) {
                return Err("HTTP server exited before readiness".into());
            }

            let client = IpcHttpClient::with_config(&self.endpoint, client_config())?;
            if let Ok(Ok(response)) = timeout(Duration::from_millis(250), client.get("/ready").send()).await {
                if response.is_success() {
                    return Ok(());
                }
            }

            if Instant::now() >= deadline {
                return Err(format!("HTTP server readiness timed out for {:?}", self.endpoint).into());
            }
            tokio::task::yield_now().await;
        }
    }

    async fn stop(mut self) -> TestResult {
        if let Some(task) = self.task.take() {
            task.abort();
            match task.await {
                Err(error) if error.is_cancelled() => {}
                Err(error) => return Err(error.into()),
                Ok(Err(error)) => return Err(error.into()),
                Ok(Ok(())) => {}
            }
        }

        #[cfg(unix)]
        wait_until_path_removed(&self.endpoint).await?;
        Ok(())
    }
}

impl Drop for HttpServerGuard {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(unix)]
async fn wait_until_path_removed(path: &Path) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(2);
    while path.exists() {
        if Instant::now() >= deadline {
            return Err(format!("IPC socket was not removed after listener drop: {path:?}").into());
        }
        tokio::task::yield_now().await;
    }
    Ok(())
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

async fn read_status(stream: &mut RawStream) -> TestResult {
    let mut response = [0u8; 128];
    let bytes_read = timeout(REQUEST_TIMEOUT, stream.read(&mut response)).await??;
    if !response[..bytes_read].starts_with(b"HTTP/1.1 200") {
        return Err(format!("unexpected raw HTTP response: {:?}", &response[..bytes_read]).into());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fragmented_headers_dispatch_only_after_the_terminator() -> TestResult {
    let endpoint = unique_endpoint("fragmented-header");
    let dispatched = Arc::new(Notify::new());
    let handler_dispatched = Arc::clone(&dispatched);
    let router = Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/fragmented", move |_ctx| {
            let dispatched = Arc::clone(&handler_dispatched);
            async move {
                dispatched.notify_one();
                Ok(HttpResponse::text("ok"))
            }
        });
    let server = HttpServerGuard::start(endpoint.clone(), server_config(4), router).await?;

    let mut stream = raw_connect(&endpoint).await?;
    stream
        .write_all(b"GET /fragmented HTTP/1.1\r\nHost: timing")
        .await?;
    stream.flush().await?;
    assert!(timeout(Duration::from_millis(50), dispatched.notified())
        .await
        .is_err());

    stream.write_all(b"\r\n\r\n").await?;
    stream.flush().await?;
    timeout(REQUEST_TIMEOUT, dispatched.notified()).await?;
    read_status(&mut stream).await?;

    drop(stream);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_reader_with_a_spare_permit_does_not_block_a_healthy_client() -> TestResult {
    let endpoint = unique_endpoint("slow-reader");
    let response_started = Arc::new(Notify::new());
    let handler_started = Arc::clone(&response_started);
    let large_response = Bytes::from(vec![b'x'; 2 * 1024 * 1024]);
    let router = Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/large", move |_ctx| {
            let response_started = Arc::clone(&handler_started);
            let large_response = large_response.clone();
            async move {
                response_started.notify_one();
                Ok(HttpResponse::builder().body(large_response).build())
            }
        });
    let server = HttpServerGuard::start(endpoint.clone(), server_config(2), router).await?;

    let mut slow_reader = raw_connect(&endpoint).await?;
    slow_reader
        .write_all(b"GET /large HTTP/1.1\r\nHost: timing\r\n\r\n")
        .await?;
    slow_reader.flush().await?;
    timeout(REQUEST_TIMEOUT, response_started.notified()).await?;

    let healthy = IpcHttpClient::with_config(&endpoint, client_config())?;
    let response = timeout(REQUEST_TIMEOUT, healthy.get("/ready").send()).await??;
    assert!(response.is_success());

    drop(slow_reader);
    drop(healthy);
    server.stop().await
}

#[ignore = "candidate acceptance: a bounded response write must release the only connection permit"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_reader_timeout_releases_the_only_connection_permit() -> TestResult {
    let endpoint = unique_endpoint("slow-reader-capacity");
    let response_started = Arc::new(Notify::new());
    let handler_started = Arc::clone(&response_started);
    let large_response = Bytes::from(vec![b'x'; 2 * 1024 * 1024]);
    let router = Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/large", move |_ctx| {
            let response_started = Arc::clone(&handler_started);
            let large_response = large_response.clone();
            async move {
                response_started.notify_one();
                Ok(HttpResponse::builder().body(large_response).build())
            }
        });
    let mut config = server_config(1);
    config.write_timeout = Duration::from_millis(50);
    let server = HttpServerGuard::start(endpoint.clone(), config, router).await?;

    let mut slow_reader = raw_connect(&endpoint).await?;
    slow_reader
        .write_all(b"GET /large HTTP/1.1\r\nHost: timing\r\n\r\n")
        .await?;
    slow_reader.flush().await?;
    timeout(REQUEST_TIMEOUT, response_started.notified()).await?;

    let healthy = IpcHttpClient::with_config(&endpoint, client_config())?;
    let response = timeout(Duration::from_secs(1), healthy.get("/ready").send()).await??;
    assert!(response.is_success());

    drop(slow_reader);
    drop(healthy);
    server.stop().await
}

#[tokio::test]
async fn retry_first_success_has_no_backoff_or_extra_attempt() -> TestResult {
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let executor = RetryExecutor::new(
        RetryConfig::new()
            .max_attempts(3)
            .base_delay(Duration::from_secs(1))
            .jitter(JitterStrategy::None),
    );

    let result = executor
        .execute(move || {
            let attempts = Arc::clone(&operation_attempts);
            async move {
                attempts.fetch_add(1, Ordering::Relaxed);
                Ok::<_, KodeBridgeError>("first-success")
            }
        })
        .await?;

    assert_eq!(result, "first-success");
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
    Ok(())
}
