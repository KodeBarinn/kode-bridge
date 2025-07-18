use std::path::Path;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::AnyResult;
use crate::http_client::http_request;
use crate::stream_client::{StreamingResponse, http_request_stream};
use crate::types::Response;

/// Generic IPC HTTP client that works on both Unix and Windows platforms
pub struct IpcHttpClient {
    name: Name<'static>,
}

impl IpcHttpClient {
    /// Create a new IPC HTTP client with the given socket/named pipe path
    ///
    /// # Arguments
    /// * `path` - The path to the socket (Unix) or named pipe (Windows)
    pub fn new<P: AsRef<Path>>(path: P) -> AnyResult<Self> {
        let name = path.as_ref().to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Self { name })
    }

    /// Send an HTTP request over the IPC connection
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `path` - Request path
    /// * `body` - Optional JSON body
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let stream = LocalSocketStream::connect(self.name.clone()).await?;
        http_request(stream, method, path, body).await
    }

    /// Send a streaming HTTP request over the IPC connection
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `path` - Request path
    /// * `body` - Optional JSON body
    pub async fn request_stream(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<StreamingResponse> {
        let stream = LocalSocketStream::connect(self.name.clone()).await?;
        http_request_stream(stream, method, path, body).await
    }
}
