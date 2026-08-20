#![allow(clippy::panic)]

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use futures::future::join_all;
use kode_bridge::ipc_http_client::{ClientConfig, IpcHttpClient};
use kode_bridge::ipc_http_server::{HttpResponse, IpcHttpServer, Router, ServerConfig};
use kode_bridge::pool::PoolConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};

const SMALL_PAYLOAD_SIZE: usize = 256;
const LARGE_PAYLOAD_SIZE: usize = 512 * 1024;
const BURST_SIZE: usize = 32;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

static ENDPOINT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct BenchContext {
    runtime: Runtime,
    endpoint: PathBuf,
    server_task: Option<JoinHandle<kode_bridge::Result<()>>>,
    direct_client: IpcHttpClient,
    pooled_client: IpcHttpClient,
    small_payload: Value,
    large_payload: Value,
}

impl BenchContext {
    fn new() -> Self {
        let runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => panic!("failed to create benchmark runtime: {error}"),
        };
        let endpoint = unique_endpoint();
        let router = Router::new()
            .get("/ready", |_ctx| async { Ok(HttpResponse::text("ready")) })
            .get("/small", |_ctx| async {
                Ok(HttpResponse::builder()
                    .header("content-type", "application/json")
                    .body(Bytes::from_static(b"{\"ok\":true}"))
                    .build())
            })
            .post("/echo", |ctx| async move {
                Ok(HttpResponse::builder()
                    .header("content-type", "application/json")
                    .body(ctx.body)
                    .build())
            });
        let mut server = match IpcHttpServer::with_config(
            &endpoint,
            ServerConfig {
                max_connections: BURST_SIZE + 16,
                read_timeout: Duration::from_secs(5),
                write_timeout: Duration::from_secs(5),
                max_request_size: 2 * 1024 * 1024,
                max_header_size: 16 * 1024,
                enable_logging: false,
                max_requests_per_connection: usize::MAX,
                shutdown_timeout: Duration::from_secs(1),
            },
        ) {
            Ok(server) => server.router(router),
            Err(error) => panic!("failed to configure benchmark server: {error}"),
        };
        let server_task = runtime.spawn(async move { server.serve().await });

        let direct_client = build_client(&endpoint, false);
        runtime.block_on(wait_until_ready(&endpoint, &server_task));
        let pooled_client = build_client(&endpoint, true);

        Self {
            runtime,
            endpoint,
            server_task: Some(server_task),
            direct_client,
            pooled_client,
            small_payload: payload(SMALL_PAYLOAD_SIZE),
            large_payload: payload(LARGE_PAYLOAD_SIZE),
        }
    }
}

impl Drop for BenchContext {
    fn drop(&mut self) {
        self.direct_client.close();
        self.pooled_client.close();
        if let Some(task) = self.server_task.take() {
            task.abort();
            let result = self.runtime.block_on(task);
            assert!(result.is_err_and(|error| error.is_cancelled()));
        }
    }
}

fn unique_endpoint() -> PathBuf {
    let sequence = ENDPOINT_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    #[cfg(unix)]
    {
        PathBuf::from(format!("/tmp/kb-bench-{}-{sequence}.sock", std::process::id()))
    }

    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\kb-bench-{}-{sequence}", std::process::id()))
    }
}

const fn client_config(enable_pooling: bool) -> ClientConfig {
    ClientConfig {
        default_timeout: Duration::from_secs(5),
        pool_config: PoolConfig {
            max_size: BURST_SIZE + 16,
            min_idle: 0,
            max_idle_time_ms: 120_000,
            connection_timeout_ms: 3_000,
            retry_delay_ms: 5,
            max_retries: 3,
            max_concurrent_requests: BURST_SIZE + 16,
            max_requests_per_second: None,
        },
        enable_pooling,
        max_retries: 3,
        retry_delay: Duration::from_millis(5),
        max_concurrent_requests: BURST_SIZE + 16,
        max_requests_per_second: None,
        require_windows_server_system: false,
        #[cfg(windows)]
        windows_server_pid_verifier: None,
    }
}

fn build_client(path: &Path, enable_pooling: bool) -> IpcHttpClient {
    match IpcHttpClient::with_config(path, client_config(enable_pooling)) {
        Ok(client) => client,
        Err(error) => panic!("failed to build benchmark client: {error}"),
    }
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
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn payload(size: usize) -> Value {
    json!({"payload": "x".repeat(size)})
}

fn bench_connection_costs(c: &mut Criterion, context: &BenchContext) {
    let mut group = c.benchmark_group("ipc_transport_connection");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(40);

    group.bench_function("cold_client_get", |bencher| {
        let endpoint = context.endpoint.clone();
        bencher.to_async(&context.runtime).iter_batched(
            || build_client(&endpoint, false),
            |client| async move {
                let response = match client.get("/small").send().await {
                    Ok(response) => response,
                    Err(error) => panic!("cold benchmark request failed: {error}"),
                };
                assert!(response.is_success());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("direct_get", |bencher| {
        bencher.to_async(&context.runtime).iter(|| async {
            let response = match context.direct_client.get("/small").send().await {
                Ok(response) => response,
                Err(error) => panic!("direct benchmark request failed: {error}"),
            };
            assert!(response.is_success());
        });
    });

    context
        .runtime
        .block_on(context.pooled_client.preheat_for_puts(BURST_SIZE));

    group.bench_function("warm_pooled_get", |bencher| {
        bencher.to_async(&context.runtime).iter(|| async {
            let response = match context.pooled_client.get("/small").send().await {
                Ok(response) => response,
                Err(error) => panic!("pooled benchmark request failed: {error}"),
            };
            assert!(response.is_success());
        });
    });

    group.finish();
}

fn bench_payload_round_trip(c: &mut Criterion, context: &BenchContext) {
    let mut group = c.benchmark_group("ipc_transport_payload");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(30);

    for (name, value, size) in [
        ("small", &context.small_payload, SMALL_PAYLOAD_SIZE),
        ("large", &context.large_payload, LARGE_PAYLOAD_SIZE),
    ] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("pooled_echo", name), value, |bencher, payload| {
            bencher.to_async(&context.runtime).iter(|| async {
                let response = match context
                    .pooled_client
                    .post("/echo")
                    .json_body(payload)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => panic!("payload benchmark request failed: {error}"),
                };
                assert!(response.is_success());
            });
        });
    }

    group.finish();
}

fn bench_concurrent_burst(c: &mut Criterion, context: &BenchContext) {
    let mut group = c.benchmark_group("ipc_transport_concurrency");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(30);
    group.throughput(Throughput::Elements(BURST_SIZE as u64));

    group.bench_function(BenchmarkId::new("pooled_get_burst", BURST_SIZE), |bencher| {
        bencher.to_async(&context.runtime).iter(|| async {
            let requests = (0..BURST_SIZE).map(|_| context.pooled_client.get("/small").send());
            for result in join_all(requests).await {
                match result {
                    Ok(response) => assert!(response.is_success()),
                    Err(error) => panic!("concurrent benchmark request failed: {error}"),
                }
            }
        });
    });

    group.finish();
}

fn benchmark_ipc_transport(c: &mut Criterion) {
    let context = BenchContext::new();
    bench_connection_costs(c, &context);
    bench_payload_round_trip(c, &context);
    bench_concurrent_burst(c, &context);
}

criterion_group!(benches, benchmark_ipc_transport);
criterion_main!(benches);
