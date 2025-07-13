#![cfg(windows)]

use dotenv::dotenv;
use std::env;
use tokio::runtime::Runtime;

use criterion::{Criterion, criterion_group, criterion_main};
use kode_bridge::ipc_cilent::IpcHttpClient;
use kode_bridge::types::Response;

async fn bench_version_once(client: &IpcHttpClient) -> Response {
    let result = client.request("GET", "/version", None).await.unwrap();
    result
}

fn bench_version(c: &mut Criterion) {
    dotenv().ok();
    let rt = Runtime::new().unwrap();
    let pipe_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\mihomo".to_string());

    let mut group = c.benchmark_group("ipc_http_version");
    group.sample_size(10); // Reduce sample size to avoid connection issues
    group.bench_function("version_once", |b| {
        b.iter(|| {
            rt.block_on(async {
                let client = IpcHttpClient::new(&pipe_path).unwrap();
                let result = bench_version_once(&client).await;
                result
            });
        });
    });
    group.finish();
}

criterion_group!(benches, bench_version);
criterion_main!(benches);
