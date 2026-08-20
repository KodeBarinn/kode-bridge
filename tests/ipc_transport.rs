#![cfg(all(feature = "client", feature = "server"))]

use bytes::Bytes;
use futures::future::join_all;
use http::Method;
use kode_bridge::ipc_http_client::{ClientConfig, IpcHttpClient};
use kode_bridge::ipc_http_server::{HttpResponse, IpcHttpServer, Router, ServerConfig};
use kode_bridge::ipc_stream_client::{IpcStreamClient, StreamClientConfig};
use kode_bridge::ipc_stream_server::{IpcStreamServer, StreamMessage, StreamServerConfig, StreamSource};
use kode_bridge::pool::PoolConfig;
use kode_bridge::Result;
use serde_json::{json, Value};
use std::error::Error;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
#[cfg(windows)]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};
use tokio_stream::StreamExt as _;

type TestResult<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const LARGE_BODY_SIZE: usize = 512 * 1024;
const CONCURRENT_CLIENTS: usize = 32;

static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn unique_endpoint(label: &str) -> PathBuf {
    let sequence = ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    #[cfg(unix)]
    {
        PathBuf::from(format!("/tmp/kb-{}-{}-{}.sock", std::process::id(), sequence, label))
    }

    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\kb-{}-{}-{}", std::process::id(), sequence, label))
    }
}

const fn client_config(enable_pooling: bool) -> ClientConfig {
    ClientConfig {
        default_timeout: REQUEST_TIMEOUT,
        pool_config: PoolConfig {
            max_size: CONCURRENT_CLIENTS + 8,
            min_idle: 0,
            max_idle_time_ms: 30_000,
            connection_timeout_ms: 1_000,
            retry_delay_ms: 5,
            max_retries: 3,
            max_concurrent_requests: CONCURRENT_CLIENTS + 8,
            max_requests_per_second: None,
        },
        enable_pooling,
        max_retries: 3,
        retry_delay: Duration::from_millis(5),
        max_concurrent_requests: CONCURRENT_CLIENTS + 8,
        max_requests_per_second: None,
        require_windows_server_system: false,
        #[cfg(windows)]
        windows_server_pid_verifier: None,
    }
}

fn direct_client(path: &Path) -> TestResult<IpcHttpClient> {
    Ok(IpcHttpClient::with_config(path, client_config(false))?)
}

fn pooled_client(path: &Path) -> TestResult<IpcHttpClient> {
    Ok(IpcHttpClient::with_config(path, client_config(true))?)
}

fn test_router() -> Router {
    Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/method", |ctx| async move {
            HttpResponse::json(&json!({
                "method": ctx.method.as_str(),
                "connection_id": ctx.client_info.connection_id,
            }))
        })
        .post("/method", |ctx| async move {
            HttpResponse::json(&json!({
                "method": ctx.method.as_str(),
                "body": ctx.json::<Value>()?,
            }))
        })
        .put("/method", |ctx| async move {
            HttpResponse::json(&json!({
                "method": ctx.method.as_str(),
                "body": ctx.json::<Value>()?,
            }))
        })
        .delete("/method", |ctx| async move {
            HttpResponse::json(&json!({"method": ctx.method.as_str()}))
        })
        .post("/echo", |ctx| async move {
            let seen = ctx
                .headers
                .get("x-kode-test")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("missing")
                .to_owned();
            Ok(HttpResponse::builder()
                .header("content-type", "application/json")
                .header("x-kode-seen", seen)
                .body(ctx.body)
                .build())
        })
        .get("/connection", |ctx| async move {
            HttpResponse::json(&json!({"connection_id": ctx.client_info.connection_id}))
        })
        .get("/identity", |ctx| async move {
            HttpResponse::json(&json!({
                "connection_id": ctx.client_info.connection_id,
                "uid": ctx.client_info.peer_credentials.uid,
                "gid": ctx.client_info.peer_credentials.gid,
            }))
        })
        .get("/stream", |_ctx| async {
            Ok(HttpResponse::builder()
                .header("content-type", "application/x-ndjson")
                .body(Bytes::from_static(b"{\"sequence\":1}\n{\"sequence\":2}\n"))
                .build())
        })
}

struct HttpServerGuard {
    endpoint: PathBuf,
    task: Option<JoinHandle<Result<()>>>,
}

impl HttpServerGuard {
    async fn start(endpoint: PathBuf, config: ServerConfig, router: Router) -> TestResult<Self> {
        let server = IpcHttpServer::with_config(&endpoint, config)?.router(router);
        Self::start_server(endpoint, server).await
    }

    async fn start_server(endpoint: PathBuf, mut server: IpcHttpServer) -> TestResult<Self> {
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
                let task = self.task.take().ok_or("missing HTTP IPC server task")?;
                return match task.await {
                    Ok(Ok(())) => Err("HTTP IPC server exited before becoming ready".into()),
                    Ok(Err(error)) => Err(format!("HTTP IPC server failed before readiness: {error}").into()),
                    Err(error) => Err(error.into()),
                };
            }

            let client = direct_client(&self.endpoint)?;
            if let Ok(Ok(response)) = timeout(Duration::from_millis(250), client.get("/ready").send()).await {
                if response.is_success() && response.body()? == "ready" {
                    return Ok(());
                }
            }

            if Instant::now() >= deadline {
                return Err(format!("HTTP IPC server did not become ready at {:?}", self.endpoint).into());
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
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
            return Err(format!("IPC socket path was not removed after listener drop: {path:?}").into());
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

const fn server_config() -> ServerConfig {
    ServerConfig {
        max_connections: CONCURRENT_CLIENTS + 8,
        read_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(2),
        max_request_size: 2 * 1024 * 1024,
        max_header_size: 16 * 1024,
        enable_logging: false,
        max_requests_per_connection: 64,
        shutdown_timeout: Duration::from_secs(1),
    }
}

fn response_json(response: kode_bridge::ipc_http_client::HttpResponse) -> TestResult<Value> {
    Ok(response.json_value()?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_methods_headers_and_payloads_round_trip() -> TestResult {
    let endpoint = unique_endpoint("http-roundtrip");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let client = pooled_client(&endpoint)?;

    let get = response_json(client.get("/method").send().await?)?;
    assert_eq!(get["method"], "GET");

    let post_body = json!({"kind": "post", "value": 7});
    let post = response_json(client.post("/method").json_body(&post_body).send().await?)?;
    assert_eq!(post["method"], "POST");
    assert_eq!(post["body"], post_body);

    let put_body = json!({"kind": "put", "value": 11});
    let put = response_json(client.put("/method").json_body(&put_body).send().await?)?;
    assert_eq!(put["method"], "PUT");
    assert_eq!(put["body"], put_body);

    let delete = response_json(client.delete("/method").send().await?)?;
    assert_eq!(delete["method"], "DELETE");

    let small_body = json!({"payload": "small"});
    let small = client
        .post("/echo")
        .header("x-kode-test", "header-roundtrip")
        .json_body(&small_body)
        .send()
        .await?;
    assert_eq!(small.headers()["x-kode-seen"], "header-roundtrip");
    assert_eq!(small.json_value()?, small_body);

    let large_body = json!({"payload": "x".repeat(LARGE_BODY_SIZE)});
    let large = client
        .post("/echo")
        .header("x-kode-test", "large")
        .json_body(&large_body)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    assert_eq!(large.json_value()?, large_body);

    drop(client);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pooled_connections_are_preheated_reused_and_closed() -> TestResult {
    let endpoint = unique_endpoint("pool");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let client = pooled_client(&endpoint)?;

    client.preheat_for_puts(4).await;
    let preheated = client.pool_stats().ok_or("pooling unexpectedly disabled")?;
    assert_eq!(preheated.total_connections, 4);
    assert_eq!(preheated.active_connections, 0);

    let mut connection_ids = Vec::new();
    for _ in 0..5 {
        let response = response_json(client.get("/connection").send().await?)?;
        connection_ids.push(response["connection_id"].clone());
    }
    assert_eq!(connection_ids[0], connection_ids[4]);
    assert_ne!(connection_ids[0], connection_ids[1]);

    client.close();
    let closed = client.pool_stats().ok_or("pooling unexpectedly disabled")?;
    assert_eq!(closed.total_connections, 0);
    assert_eq!(closed.active_connections, 0);

    drop(client);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pooled_connection_reaches_server_request_limit() -> TestResult {
    let endpoint = unique_endpoint("request-limit");
    let mut config = server_config();
    config.max_requests_per_connection = 2;
    let server = HttpServerGuard::start(endpoint.clone(), config, test_router()).await?;
    let client = pooled_client(&endpoint)?;

    let first = response_json(client.get("/connection").send().await?)?;
    let second = response_json(client.get("/connection").send().await?)?;
    assert_eq!(first["connection_id"], second["connection_id"]);

    let replacement = response_json(client.get("/connection").send().await?)?;
    assert_ne!(replacement["connection_id"], first["connection_id"]);

    drop(client);
    server.stop().await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unix_peer_credentials_are_kernel_reported_and_stable_per_connection() -> TestResult {
    let endpoint = unique_endpoint("peer-credentials");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let client = pooled_client(&endpoint)?;

    let first = response_json(client.get("/identity").send().await?)?;
    let second = response_json(client.get("/identity").send().await?)?;

    assert_eq!(first["uid"], u64::from(unsafe { libc::geteuid() }));
    assert_eq!(first["gid"], u64::from(unsafe { libc::getegid() }));
    assert_eq!(second["uid"], first["uid"]);
    assert_eq!(second["gid"], first["gid"]);
    assert_eq!(second["connection_id"], first["connection_id"]);

    drop(client);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_and_duplicate_listeners_fail_without_hanging() -> TestResult {
    let missing_endpoint = unique_endpoint("missing");
    let missing_client = direct_client(&missing_endpoint)?;
    let missing_result = timeout(REQUEST_TIMEOUT, missing_client.get("/ready").send()).await?;
    drop(missing_client);
    assert!(missing_result.is_err());

    let endpoint = unique_endpoint("duplicate");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let mut duplicate = IpcHttpServer::with_config(&endpoint, server_config())?.router(test_router());
    let duplicate_result = timeout(Duration::from_secs(1), duplicate.serve()).await?;
    assert!(duplicate_result.is_err());

    let client = direct_client(&endpoint)?;
    assert!(client.get("/ready").send().await?.is_success());
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_timeout_does_not_block_follow_up_request() -> TestResult {
    let endpoint = unique_endpoint("request-timeout");
    let router = Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/pending", |_ctx| async {
            std::future::pending::<Result<HttpResponse>>().await
        });
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), router).await?;
    let client = direct_client(&endpoint)?;

    let result = timeout(
        Duration::from_secs(2),
        client
            .get("/pending")
            .timeout(Duration::from_millis(25))
            .send(),
    )
    .await?;
    let error = match result {
        Ok(_) => return Err("pending request unexpectedly succeeded".into()),
        Err(error) => error,
    };
    let message = error.to_string().to_ascii_lowercase();
    assert!(message.contains("timeout") || message.contains("timed out"));

    let response = timeout(Duration::from_secs(1), client.get("/ready").send()).await??;
    assert!(response.is_success());
    drop(client);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_waiting_request_does_not_block_new_client() -> TestResult {
    let endpoint = unique_endpoint("request-cancel");
    let handler_entered = Arc::new(Notify::new());
    let pending_entered = Arc::clone(&handler_entered);
    let router = Router::new()
        .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
        .get("/pending", move |_ctx| {
            let pending_entered = Arc::clone(&pending_entered);
            async move {
                pending_entered.notify_one();
                std::future::pending::<Result<HttpResponse>>().await
            }
        });
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), router).await?;

    let waiting_endpoint = endpoint.clone();
    let waiting_request = tokio::spawn(async move {
        let client = direct_client(&waiting_endpoint)?;
        let result = client.get("/pending").send().await;
        drop(client);
        match result {
            Ok(response) => TestResult::Ok(response),
            Err(error) => Err(error.into()),
        }
    });
    timeout(Duration::from_secs(1), handler_entered.notified()).await?;

    waiting_request.abort();
    let cancelled = timeout(Duration::from_secs(1), waiting_request).await?;
    assert!(cancelled.is_err_and(|error| error.is_cancelled()));

    let client = direct_client(&endpoint)?;
    let response = timeout(Duration::from_secs(1), client.get("/ready").send()).await??;
    assert!(response.is_success());
    drop(client);
    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn thirty_two_clients_connect_concurrently() -> TestResult {
    let endpoint = unique_endpoint("concurrent");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;

    let requests = (0..CONCURRENT_CLIENTS).map(|_| {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let client = direct_client(&endpoint)?;
            let response = client.get("/method").send().await?;
            drop(client);
            if response.status() != 200 || response.json_value()?["method"] != "GET" {
                return Err("unexpected concurrent response".into());
            }
            TestResult::Ok(())
        })
    });

    for result in join_all(requests).await {
        result??;
    }

    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_client_reads_lines_disconnects_and_reconnects() -> TestResult {
    let endpoint = unique_endpoint("stream-client");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let config = StreamClientConfig {
        default_timeout: REQUEST_TIMEOUT,
        max_retries: 3,
        retry_delay: Duration::from_millis(5),
        buffer_size: 8192,
    };

    let client = IpcStreamClient::with_config(&endpoint, config.clone())?;
    let mut response = client.get("/stream").send().await?.into_inner();
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.next().await.ok_or("missing first stream line")??,
        r#"{"sequence":1}"#
    );
    drop(response);

    let reconnected = IpcStreamClient::with_config(&endpoint, config)?;
    let mut response = reconnected.get("/stream").send().await?.into_inner();
    assert_eq!(
        response
            .next()
            .await
            .ok_or("missing reconnected stream line")??,
        r#"{"sequence":1}"#
    );
    assert_eq!(
        response
            .next()
            .await
            .ok_or("missing second stream line")??,
        r#"{"sequence":2}"#
    );
    drop(response);

    server.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn streaming_clients_connect_concurrently() -> TestResult {
    let endpoint = unique_endpoint("stream-concurrent");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;

    let requests = (0..8).map(|_| {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let client = IpcStreamClient::with_config(
                &endpoint,
                StreamClientConfig {
                    default_timeout: REQUEST_TIMEOUT,
                    max_retries: 3,
                    retry_delay: Duration::from_millis(5),
                    buffer_size: 8192,
                },
            )?;
            let mut response = client.get("/stream").send().await?.into_inner();
            let line = response
                .next()
                .await
                .ok_or("missing concurrent stream line")??;
            if line != r#"{"sequence":1}"# {
                return Err("unexpected concurrent stream response".into());
            }
            TestResult::Ok(())
        })
    });

    for result in join_all(requests).await {
        result??;
    }
    server.stop().await
}

struct ReadySource {
    ready: Option<oneshot::Sender<()>>,
}

impl StreamSource for ReadySource {
    fn next_messages(&mut self) -> Pin<Box<dyn Future<Output = Result<Vec<StreamMessage>>> + Send + '_>> {
        Box::pin(std::future::pending())
    }

    fn has_more(&self) -> bool {
        true
    }

    fn initialize(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            if let Some(ready) = self.ready.take() {
                let _ = ready.send(());
            }
            Ok(())
        })
    }

    fn cleanup(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

struct StreamServerGuard {
    #[cfg(unix)]
    endpoint: PathBuf,
    task: Option<JoinHandle<Result<()>>>,
}

impl StreamServerGuard {
    async fn start(endpoint: PathBuf) -> TestResult<Self> {
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut server = IpcStreamServer::with_config(
            &endpoint,
            StreamServerConfig {
                max_connections: 8,
                buffer_size: 8192,
                write_timeout: Duration::from_secs(1),
                max_message_size: 64 * 1024,
                enable_logging: false,
                shutdown_timeout: Duration::from_millis(250),
                broadcast_capacity: 16,
                keepalive_interval: Duration::from_secs(30),
            },
        )?;
        let task = tokio::spawn(async move {
            server
                .serve_with_source(ReadySource { ready: Some(ready_tx) })
                .await
        });
        timeout(STARTUP_TIMEOUT, ready_rx).await??;
        Ok(Self {
            #[cfg(unix)]
            endpoint,
            task: Some(task),
        })
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

impl Drop for StreamServerGuard {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_server_rejects_duplicate_listener_and_can_restart() -> TestResult {
    let endpoint = unique_endpoint("stream-server");
    let server = StreamServerGuard::start(endpoint.clone()).await?;

    let (ready_tx, _ready_rx) = oneshot::channel();
    let mut duplicate = IpcStreamServer::new(&endpoint)?;
    let duplicate_result = timeout(
        Duration::from_secs(1),
        duplicate.serve_with_source(ReadySource { ready: Some(ready_tx) }),
    )
    .await?;
    assert!(duplicate_result.is_err());

    server.stop().await?;
    StreamServerGuard::start(endpoint).await?.stop().await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unix_stale_socket_and_drop_cleanup_are_preserved() -> TestResult {
    let stale_endpoint = unique_endpoint("unix-stale");
    let stale_listener = std::os::unix::net::UnixListener::bind(&stale_endpoint)?;
    drop(stale_listener);
    assert!(stale_endpoint.exists());

    let mut stale_server = IpcHttpServer::with_config(&stale_endpoint, server_config())?.router(test_router());
    let stale_result = timeout(Duration::from_secs(1), stale_server.serve()).await?;
    assert!(stale_result.is_err());
    assert!(stale_endpoint.exists());
    std::fs::remove_file(stale_endpoint)?;

    let cleanup_endpoint = unique_endpoint("unix-cleanup");
    let server = HttpServerGuard::start(cleanup_endpoint.clone(), server_config(), test_router()).await?;
    assert!(cleanup_endpoint.exists());

    server.stop().await?;
    assert!(!cleanup_endpoint.exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unix_listener_mode_is_applied() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let endpoint = unique_endpoint("unix-mode");
    let server = IpcHttpServer::with_config(&endpoint, server_config())?
        .with_listener_mode(0o640)
        .router(test_router());
    let server = HttpServerGuard::start_server(endpoint.clone(), server).await?;
    let mode = std::fs::metadata(&endpoint)?.permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
    server.stop().await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unix_listener_does_not_replace_a_regular_file() -> TestResult {
    let endpoint = unique_endpoint("regular-file");
    std::fs::write(&endpoint, b"sentinel")?;

    let mut server = IpcHttpServer::with_config(&endpoint, server_config())?.router(test_router());
    let serve_result = timeout(Duration::from_secs(1), server.serve()).await?;
    assert!(serve_result.is_err());
    assert_eq!(std::fs::read(&endpoint)?, b"sentinel");
    std::fs::remove_file(endpoint)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn unix_endpoints_with_nul_are_rejected_during_construction() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let endpoint = PathBuf::from(OsString::from_vec(b"/tmp/kb-invalid\0.sock".to_vec()));
    assert!(IpcHttpClient::new(&endpoint).is_err());
    assert!(IpcHttpServer::new(endpoint).is_err());
}

#[cfg(windows)]
#[test]
fn windows_non_pipe_endpoints_are_rejected_during_construction() {
    let endpoint = PathBuf::from(r"C:\temp\kode-bridge-invalid.sock");
    assert!(IpcHttpClient::new(&endpoint).is_err());
    assert!(IpcHttpServer::new(endpoint).is_err());
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn windows_sddl_listener_accepts_thirty_two_clients() -> TestResult {
    let endpoint = unique_endpoint("windows-sddl");
    let server = IpcHttpServer::with_config(&endpoint, server_config())?
        .with_listener_security_descriptor("D:(A;;GA;;;WD)")
        .router(test_router());
    let server = HttpServerGuard::start_server(endpoint.clone(), server).await?;

    let requests = (0..CONCURRENT_CLIENTS).map(|_| {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let client = direct_client(&endpoint)?;
            let response = client.get("/ready").send().await?;
            drop(client);
            if !response.is_success() {
                return Err("unexpected Windows SDDL response".into());
            }
            TestResult::Ok(())
        })
    });
    for result in join_all(requests).await {
        result??;
    }

    server.stop().await
}

#[cfg(windows)]
static WINDOWS_VERIFIER_SUCCESS_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static WINDOWS_VERIFIER_FAILURE_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static WINDOWS_VERIFIER_REUSE_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static WINDOWS_VERIFIER_RECONNECT_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static WINDOWS_VERIFIER_PUT_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(windows)]
fn verify_current_process(process_id: u32, calls: &AtomicUsize) -> std::io::Result<()> {
    calls.fetch_add(1, Ordering::SeqCst);
    if process_id == std::process::id() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "named-pipe server PID did not match the test server",
        ))
    }
}

#[cfg(windows)]
fn accept_windows_server(process_id: u32) -> std::io::Result<()> {
    verify_current_process(process_id, &WINDOWS_VERIFIER_SUCCESS_CALLS)
}

#[cfg(windows)]
fn reject_windows_server(_process_id: u32) -> std::io::Result<()> {
    WINDOWS_VERIFIER_FAILURE_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "injected server identity rejection",
    ))
}

#[cfg(windows)]
fn accept_windows_server_for_reuse(process_id: u32) -> std::io::Result<()> {
    verify_current_process(process_id, &WINDOWS_VERIFIER_REUSE_CALLS)
}

#[cfg(windows)]
fn accept_windows_server_for_reconnect(process_id: u32) -> std::io::Result<()> {
    verify_current_process(process_id, &WINDOWS_VERIFIER_RECONNECT_CALLS)
}

#[cfg(windows)]
fn accept_windows_server_for_put_fallback(process_id: u32) -> std::io::Result<()> {
    verify_current_process(process_id, &WINDOWS_VERIFIER_PUT_FALLBACK_CALLS)
}

#[cfg(windows)]
fn windows_verified_client_config(enable_pooling: bool, verifier: fn(u32) -> std::io::Result<()>) -> ClientConfig {
    let mut config = client_config(enable_pooling);
    config.max_retries = 1;
    config.pool_config.max_retries = 1;
    config.windows_server_pid_verifier = Some(verifier);
    config
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_server_pid_verifier_accepts_before_request() -> TestResult {
    WINDOWS_VERIFIER_SUCCESS_CALLS.store(0, Ordering::SeqCst);
    let endpoint = unique_endpoint("windows-verifier-success");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let config = windows_verified_client_config(false, accept_windows_server);
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    assert!(client.get("/ready").send().await?.is_success());
    assert_eq!(WINDOWS_VERIFIER_SUCCESS_CALLS.load(Ordering::SeqCst), 1);

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_server_pid_verifier_rejects_before_request() -> TestResult {
    WINDOWS_VERIFIER_FAILURE_CALLS.store(0, Ordering::SeqCst);
    let endpoint = unique_endpoint("windows-verifier-failure");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let config = windows_verified_client_config(false, reject_windows_server);
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    assert!(client.get("/ready").send().await.is_err());
    assert_eq!(WINDOWS_VERIFIER_FAILURE_CALLS.load(Ordering::SeqCst), 1);

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_server_pid_verifier_runs_once_for_pooled_connection() -> TestResult {
    WINDOWS_VERIFIER_REUSE_CALLS.store(0, Ordering::SeqCst);
    let endpoint = unique_endpoint("windows-verifier-reuse");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let config = windows_verified_client_config(true, accept_windows_server_for_reuse);
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    let first = response_json(client.get("/connection").send().await?)?;
    let second = response_json(client.get("/connection").send().await?)?;
    assert_eq!(first["connection_id"], second["connection_id"]);
    assert_eq!(WINDOWS_VERIFIER_REUSE_CALLS.load(Ordering::SeqCst), 1);

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_server_pid_verifier_runs_again_after_reconnect() -> TestResult {
    WINDOWS_VERIFIER_RECONNECT_CALLS.store(0, Ordering::SeqCst);
    let endpoint = unique_endpoint("windows-verifier-reconnect");
    let mut server_settings = server_config();
    server_settings.max_requests_per_connection = 1;
    let server = HttpServerGuard::start(endpoint.clone(), server_settings, test_router()).await?;
    let mut config = windows_verified_client_config(true, accept_windows_server_for_reconnect);
    config.max_retries = 3;
    config.pool_config.max_retries = 3;
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    let first = response_json(client.get("/connection").send().await?)?;
    let second = response_json(client.get("/connection").send().await?)?;
    assert_ne!(first["connection_id"], second["connection_id"]);
    assert_eq!(WINDOWS_VERIFIER_RECONNECT_CALLS.load(Ordering::SeqCst), 2);

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_server_pid_verifier_covers_large_put_direct_fallback() -> TestResult {
    WINDOWS_VERIFIER_PUT_FALLBACK_CALLS.store(0, Ordering::SeqCst);
    let endpoint = unique_endpoint("windows-verifier-put-fallback");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let mut config = windows_verified_client_config(true, accept_windows_server_for_put_fallback);
    config.pool_config.max_size = 1;
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    client.preheat_for_puts(1).await;
    assert_eq!(WINDOWS_VERIFIER_PUT_FALLBACK_CALLS.load(Ordering::SeqCst), 1);

    let body = Bytes::from(json!({"payload": "x".repeat(12_000)}).to_string());
    assert!(client
        .put("/method")
        .json_bytes(body)
        .send()
        .await?
        .is_success());
    assert_eq!(WINDOWS_VERIFIER_PUT_FALLBACK_CALLS.load(Ordering::SeqCst), 2);

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_local_system_requirement_rejects_user_server() -> TestResult {
    let endpoint = unique_endpoint("windows-require-system");
    let server = HttpServerGuard::start(endpoint.clone(), server_config(), test_router()).await?;
    let mut config = client_config(false);
    config.max_retries = 1;
    config.require_windows_server_system = true;
    let client = IpcHttpClient::with_config(&endpoint, config)?;

    assert!(client.get("/ready").send().await.is_err());

    drop(client);
    server.stop().await
}

#[cfg(windows)]
#[test]
#[should_panic(expected = "Failed to parse SDDL")]
#[allow(clippy::panic)]
fn windows_invalid_sddl_is_rejected() {
    let endpoint = unique_endpoint("windows-invalid-sddl");
    let server = match IpcHttpServer::new(endpoint) {
        Ok(server) => server,
        Err(error) => panic!("server construction should succeed: {error}"),
    };
    let _server = server.with_listener_security_descriptor("not-valid-sddl");
}

#[test]
fn unique_endpoints_do_not_collide() {
    let first = unique_endpoint("identity");
    let second = unique_endpoint("identity");
    assert_ne!(first, second);
}

#[test]
fn router_accepts_all_core_methods() {
    let router = test_router();
    for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
        assert!(router.find_handler_and_params(&method, "/method").is_some());
    }
}
