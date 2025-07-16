/// This example demonstrates how to use `IpcHttpClient` from the `kode_bridge` crate to send an HTTP GET request
/// over an IPC transport (Unix socket on Unix-like systems, Named Pipe on Windows).
///
/// The IPC path is determined by platform-specific environment variables:
/// - Unix: `CUSTOM_SOCK` (e.g., "/tmp/custom.sock")  
/// - Windows: `CUSTOM_PIPE` (e.g., "\\.\pipe\my_pipe")
///
/// The client sends a GET request to the `/proxies` endpoint which typically
/// returns a larger response compared to the `/version` endpoint.
///
/// # Errors
/// Returns an error if the environment variable is missing, the client fails to connect, or the request fails.
///
/// # Example
/// ```env
/// # Unix
/// CUSTOM_SOCK=/tmp/custom.sock
/// 
/// # Windows  
/// CUSTOM_PIPE=\\.\pipe\my_pipe
/// ```
use dotenv::dotenv;
use kode_bridge::IpcHttpClient;
use kode_bridge::errors::AnyError;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn AnyError>> {
    dotenv().ok();

    // Use platform-appropriate environment variable
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK")?;
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE")?;

    let client = IpcHttpClient::new(&ipc_path)?;
    let response = client.request("GET", "/proxies", None).await?;
    println!("{:?}", response);
    println!("{}", response.json()?);
    Ok(())
}
