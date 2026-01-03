/// Cross-platform benchmark for measuring IPC HTTP client performance
///
/// This benchmark measures the performance of sending HTTP requests over IPC.
/// It automatically detects the platform and uses appropriate environment variables:
/// - Unix: Uses CUSTOM_SOCK environment variable (defaults to /tmp/custom.sock)  
/// - Windows: Uses CUSTOM_PIPE environment variable (defaults to \\.\pipe\mihomo)
///
/// # Environment Variables
/// - CUSTOM_SOCK: Unix socket path (Unix only)
/// - CUSTOM_PIPE: Named pipe path (Windows only)
///
/// # Example
/// ```bash
/// # Unix
/// CUSTOM_SOCK=/tmp/my_socket cargo bench
///
/// # Windows  
/// CUSTOM_PIPE=\\.\pipe\my_pipe cargo bench
/// ```
use dotenvy::dotenv;
use std::env;
use tokio::runtime::Runtime;

use criterion::{criterion_group, criterion_main, Criterion};
use kode_bridge::http_client::Response;
use kode_bridge::IpcHttpClient;

#[allow(clippy::expect_used, clippy::unwrap_used)]
async fn bench_version_once(client: &IpcHttpClient) -> Response {
    client
        .get("/version")
        .send()
        .await
        .expect("Request failed")
        .into_inner()
}

fn get_ipc_path() -> String {
    dotenv().ok();

    #[cfg(unix)]
    {
        env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/custom.sock".to_string())
    }

    #[cfg(windows)]
    {
        env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\mihomo".to_string())
    }

    #[cfg(not(any(unix, windows)))]
    {
        env::var("CUSTOM_IPC").unwrap_or_else(|_| "default_ipc_path".to_string())
    }
}

#[allow(clippy::expect_used, clippy::unwrap_used)]
fn bench_version(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");
    let ipc_path = get_ipc_path();

    let mut group = c.benchmark_group("ipc_http_version");

    // On Windows, reduce sample size to avoid connection issues
    #[cfg(windows)]
    group.sample_size(10);

    // Create client once for Unix, or per-iteration for Windows
    let client = IpcHttpClient::new(&ipc_path).expect("Failed to create IpcHttpClient");

    group.bench_function("version_once", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..1_000 {
                    let _ = bench_version_once(&client).await;
                }
            });
        });
    });
    group.finish();
}

criterion_group!(benches, bench_version);
criterion_main!(benches);
