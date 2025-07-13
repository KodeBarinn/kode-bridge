#![cfg(unix)]

use dotenv::dotenv;
use kode_bridge::IpcHttpClient;
use kode_bridge::errors::AnyError;
use std::env;

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    dotenv().ok();

    let ipc_path = env::var("CUSTOM_SOCK")?;
    let client = IpcHttpClient::new(&ipc_path);
    let response = client.request("GET", "/proxies", None).await?;
    println!("{:?}", response);
    println!("{}", response.json()?);
    Ok(())
}
