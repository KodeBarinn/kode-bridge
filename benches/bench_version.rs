#![allow(clippy::panic)]

use bytes::{Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use http::Method;
use kode_bridge::codec::HttpIpcCodec;
use kode_bridge::http_client::{send_request, RequestBuilder};
use kode_bridge::ipc_http_server::{ClientInfo, HttpResponse, RequestContext, Router};
use kode_bridge::IpcHttpClient;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{duplex, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::runtime::Runtime;
use tokio_util::codec::{Decoder as _, Encoder as _};

const SMALL_JSON_PAYLOAD_SIZE: usize = 256;
const LARGE_JSON_PAYLOAD_SIZE: usize = 8 * 1024;
const SMALL_BATCH_SIZE: usize = 8;
const LARGE_BATCH_SIZE: usize = 32;

struct BenchContext {
    rt: Runtime,
    client: IpcHttpClient,
    small_payload: Value,
    large_payload: Value,
    small_batch: Vec<(String, Value)>,
    large_batch: Vec<(String, Value)>,
    ok_response_small: Bytes,
    ok_response_large: Bytes,
    router: Arc<Router>,
    raw_get_request: Bytes,
    raw_put_small_request: Bytes,
    raw_put_large_request: Bytes,
}

impl BenchContext {
    fn new() -> Self {
        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(err) => panic!("failed to create benchmark runtime: {err}"),
        };
        let client = match IpcHttpClient::new("/tmp/kode-bridge-bench.sock") {
            Ok(client) => client,
            Err(err) => panic!("failed to create benchmark client: {err}"),
        };

        let small_payload = build_payload(SMALL_JSON_PAYLOAD_SIZE);
        let large_payload = build_payload(LARGE_JSON_PAYLOAD_SIZE);
        let small_batch = build_batch(SMALL_BATCH_SIZE, SMALL_JSON_PAYLOAD_SIZE);
        let large_batch = build_batch(LARGE_BATCH_SIZE, SMALL_JSON_PAYLOAD_SIZE);

        let ok_response_small = encode_response(match HttpResponse::json(&json!({"ok": true})) {
            Ok(response) => response,
            Err(err) => panic!("failed to encode small benchmark response: {err}"),
        });
        let ok_response_large = encode_response(
            match HttpResponse::json(&json!({
                "ok": true,
                "payload": "x".repeat(LARGE_JSON_PAYLOAD_SIZE)
            })) {
                Ok(response) => response,
                Err(err) => panic!("failed to encode large benchmark response: {err}"),
            },
        );

        let router = Arc::new(
            Router::new()
                .get("/version", |_ctx| async move {
                    HttpResponse::json(&json!({"version": "bench"}))
                })
                .put("/config", |ctx| async move {
                    let payload = ctx.json::<Value>()?;
                    HttpResponse::json(&json!({
                        "accepted": true,
                        "payload_size": payload.to_string().len()
                    }))
                })
                .put("/batch/:id", |ctx| async move {
                    let payload = ctx.json::<Value>()?;
                    let id = ctx.path_params().get("id").cloned().unwrap_or_default();
                    HttpResponse::json(&json!({
                        "id": id,
                        "payload_size": payload.to_string().len()
                    }))
                }),
        );

        let raw_get_request = match RequestBuilder::new(Method::GET, "/version".to_string()).build() {
            Ok(request) => request,
            Err(err) => panic!("failed to build GET request: {err}"),
        };
        let raw_put_small_request = match RequestBuilder::new(Method::PUT, "/config".to_string())
            .json(&small_payload)
            .and_then(RequestBuilder::build)
        {
            Ok(request) => request,
            Err(err) => panic!("failed to build small PUT request: {err}"),
        };
        let raw_put_large_request = match RequestBuilder::new(Method::PUT, "/config".to_string())
            .json(&large_payload)
            .and_then(RequestBuilder::build)
        {
            Ok(request) => request,
            Err(err) => panic!("failed to build large PUT request: {err}"),
        };

        Self {
            rt,
            client,
            small_payload,
            large_payload,
            small_batch,
            large_batch,
            ok_response_small,
            ok_response_large,
            router,
            raw_get_request,
            raw_put_small_request,
            raw_put_large_request,
        }
    }
}

fn build_payload(target_size: usize) -> Value {
    json!({
        "name": "benchmark",
        "payload": "x".repeat(target_size),
        "enabled": true,
        "count": 42
    })
}

fn build_batch(batch_size: usize, payload_size: usize) -> Vec<(String, Value)> {
    (0..batch_size)
        .map(|index| (format!("/batch/{}", index), build_payload(payload_size)))
        .collect()
}

fn encode_response(response: HttpResponse) -> Bytes {
    let mut codec = HttpIpcCodec::new(4096, 2 * 1024 * 1024);
    let mut buffer = BytesMut::new();
    if let Err(err) = codec.encode(response, &mut buffer) {
        panic!("failed to encode response: {err}");
    }
    buffer.freeze()
}

async fn client_roundtrip_once(request: Bytes, response: Bytes) {
    let (mut client_stream, mut server_stream) = duplex(64 * 1024);
    let request_len = request.len();

    let server_task = tokio::spawn(async move {
        let mut request_buffer = vec![0u8; request_len];
        if let Err(err) = server_stream.read_exact(&mut request_buffer).await {
            panic!("failed to read benchmark request: {err}");
        }
        if let Err(err) = server_stream.write_all(&response).await {
            panic!("failed to write benchmark response: {err}");
        }
        if let Err(err) = server_stream.flush().await {
            panic!("failed to flush response: {err}");
        }
    });

    let response = match send_request(&mut client_stream, request).await {
        Ok(response) => response,
        Err(err) => panic!("client roundtrip failed: {err}"),
    };
    assert!(response.is_success());

    if let Err(err) = server_task.await {
        panic!("benchmark server task failed: {err}");
    }
}

async fn server_pipeline_once(router: Arc<Router>, codec: &mut HttpIpcCodec, request: Bytes) {
    let mut src = BytesMut::from(request.as_ref());
    let parsed = match codec.decode(&mut src) {
        Ok(Some(request)) => request,
        Ok(None) => panic!("expected complete request"),
        Err(err) => panic!("server decode failed: {err}"),
    };

    let mut context = RequestContext {
        method: parsed.method,
        uri: parsed.uri,
        path_params: HashMap::new(),
        headers: parsed.headers,
        body: parsed.body,
        client_info: ClientInfo {
            connection_id: 1,
            connected_at: Instant::now(),
        },
        timestamp: Instant::now(),
    };

    let response = if let Some((handler, params)) = router.find_handler_and_params(&context.method, context.uri.path())
    {
        context.path_params = params;
        match (handler)(context).await {
            Ok(response) => response,
            Err(err) => panic!("handler failed: {err}"),
        }
    } else {
        HttpResponse::not_found()
    };

    let mut encoded = BytesMut::new();
    if let Err(err) = codec.encode(response, &mut encoded) {
        panic!("server encode failed: {err}");
    }
    black_box(encoded);
}

fn bench_client_builder(c: &mut Criterion, ctx: &BenchContext) {
    let mut group = c.benchmark_group("client_builder");
    group.measurement_time(Duration::from_secs(6));

    group.bench_function("get_prepare", |b| {
        let client = &ctx.client;
        b.iter(|| {
            let builder = client.get("/version");
            black_box(builder);
        });
    });

    group.bench_function("put_prepare_small", |b| {
        let client = &ctx.client;
        let payload = &ctx.small_payload;
        b.iter(|| {
            let builder = client.put("/config").json_body(payload);
            black_box(builder);
        });
    });

    group.bench_function("put_prepare_large", |b| {
        let client = &ctx.client;
        let payload = &ctx.large_payload;
        b.iter(|| {
            let builder = client.put("/config").json_body(payload);
            black_box(builder);
        });
    });

    group.finish();
}

fn bench_client_roundtrip(c: &mut Criterion, ctx: &BenchContext) {
    let mut group = c.benchmark_group("client_roundtrip_duplex");
    group.measurement_time(Duration::from_secs(6));

    group.bench_function("get_small", |b| {
        let request = ctx.raw_get_request.clone();
        let response = ctx.ok_response_small.clone();
        b.to_async(&ctx.rt)
            .iter(|| client_roundtrip_once(request.clone(), response.clone()));
    });

    group.bench_function("put_small", |b| {
        let request = ctx.raw_put_small_request.clone();
        let response = ctx.ok_response_small.clone();
        b.to_async(&ctx.rt)
            .iter(|| client_roundtrip_once(request.clone(), response.clone()));
    });

    group.bench_function("put_large", |b| {
        let request = ctx.raw_put_large_request.clone();
        let response = ctx.ok_response_large.clone();
        b.to_async(&ctx.rt)
            .iter(|| client_roundtrip_once(request.clone(), response.clone()));
    });

    group.finish();
}

fn bench_server_pipeline(c: &mut Criterion, ctx: &BenchContext) {
    let mut group = c.benchmark_group("server_pipeline");
    group.measurement_time(Duration::from_secs(6));

    group.bench_function("get_small", |b| {
        let router = Arc::clone(&ctx.router);
        let request = ctx.raw_get_request.clone();
        b.to_async(&ctx.rt).iter(|| {
            let router = Arc::clone(&router);
            let request = request.clone();
            async move {
                let mut codec = HttpIpcCodec::new(4096, 2 * 1024 * 1024);
                server_pipeline_once(router, &mut codec, request).await;
            }
        });
    });

    group.bench_function("put_small", |b| {
        let router = Arc::clone(&ctx.router);
        let request = ctx.raw_put_small_request.clone();
        b.to_async(&ctx.rt).iter(|| {
            let router = Arc::clone(&router);
            let request = request.clone();
            async move {
                let mut codec = HttpIpcCodec::new(4096, 2 * 1024 * 1024);
                server_pipeline_once(router, &mut codec, request).await;
            }
        });
    });

    group.bench_function("put_large", |b| {
        let router = Arc::clone(&ctx.router);
        let request = ctx.raw_put_large_request.clone();
        b.to_async(&ctx.rt).iter(|| {
            let router = Arc::clone(&router);
            let request = request.clone();
            async move {
                let mut codec = HttpIpcCodec::new(4096, 2 * 1024 * 1024);
                server_pipeline_once(router, &mut codec, request).await;
            }
        });
    });

    group.finish();
}

fn bench_batch_prepare(c: &mut Criterion, ctx: &BenchContext) {
    let mut group = c.benchmark_group("client_batch_prepare");
    group.measurement_time(Duration::from_secs(6));

    for (name, batch) in [
        ("batch_8", &ctx.small_batch),
        ("batch_32", &ctx.large_batch),
    ] {
        group.throughput(Throughput::Elements(batch.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), batch, |b, batch| {
            let client = &ctx.client;
            b.iter(|| {
                let prepared: Vec<_> = batch
                    .iter()
                    .map(|(path, payload)| client.put(path).json_body(payload))
                    .collect();
                black_box(prepared);
            });
        });
    }

    group.finish();
}

fn benchmark_ipc_http(c: &mut Criterion) {
    let ctx = BenchContext::new();
    bench_client_builder(c, &ctx);
    bench_client_roundtrip(c, &ctx);
    bench_server_pipeline(c, &ctx);
    bench_batch_prepare(c, &ctx);
}

criterion_group!(benches, benchmark_ipc_http);
criterion_main!(benches);
