#![cfg(windows)]

/// This example demonstrates how to use `IpcHttpClient` from the `kode_bridge` crate to send an HTTP GET request
/// over an IPC transport (Windows named pipe) on Windows platform.
///
/// The named pipe path is determined by the `CUSTOM_PIPE` environment variable, which is loaded from
/// a `.env` file if present. The client sends a GET request to the `/version` endpoint and prints the raw response
/// as well as its JSON representation.
///
/// # Errors
/// Returns an error if the environment variable is missing, the client fails to connect, or the request fails.
///
/// # Example
/// ```env
/// CUSTOM_PIPE=\\.\pipe\my_pipe
/// ```
use dotenv::dotenv;
use kode_bridge::IpcHttpClient;
use kode_bridge::errors::AnyError;
use std::env;

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    dotenv().ok();

    let ipc_path = env::var("CUSTOM_PIPE")?;
    let client = IpcHttpClient::new(&ipc_path).unwrap();
    let response = client.request("GET", "/version", None).await?;
    println!("{:?}", response);
    println!("{}", response.json()?);
    Ok(())
}
