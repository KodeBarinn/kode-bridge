use dotenv::dotenv;
use std::env;
use tokio::runtime::Runtime;

use criterion::{Criterion, criterion_group, criterion_main};
use kode_bridge::ipc_cilent::IpcHttpClient;
use kode_bridge::types::Response;

async fn bench_version_once(client: &IpcHttpClient) -> Response {
    client.request("GET", "/version", None).await.unwrap()
}

fn bench_version(c: &mut Criterion) {
    dotenv().ok();
    let rt = Runtime::new().unwrap();
    let socket_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/custom.sock".to_string());
    let client = IpcHttpClient::new(&socket_path).unwrap();
    let mut group = c.benchmark_group("ipc_http_version");
    group.bench_function("version_once", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _ = bench_version_once(&client).await;
            });
        });
    });
    group.finish();
}

criterion_group!(benches, bench_version);
criterion_main!(benches);
