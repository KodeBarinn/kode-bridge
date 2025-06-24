/// This example demonstrates how to use `IpcHttpClient` from the `kode_bridge` crate to send an HTTP GET request
/// over an IPC transport (such as a Unix socket or NamedPipe, depending on the platform and environment).
///
/// The socket path or named pipe is determined by the `CUSTOM_SOCK` environment variable, which is loaded from
/// a `.env` file if present. The client sends a GET request to the `/version` endpoint and prints the raw response
/// as well as its JSON representation.
///
/// # Errors
/// Returns an error if the environment variable is missing, the client fails to connect, or the request fails.
///
/// # Example
/// ```env
/// CUSTOM_SOCK=/path/to/socket
/// ```
use dotenv::dotenv;
use kode_bridge::IpcHttpClient;
use kode_bridge::errors::AnyError;

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    dotenv().ok();

    let ipc_path = env::var("CUSTOM_SOCK")?;
    let client = IpcHttpClient::new(&ipc_path);
    let response = client.request("GET", "/version", None).await?;
    println!("{:?}", response);
    println!("{}", response.json()?);
    Ok(())
}
