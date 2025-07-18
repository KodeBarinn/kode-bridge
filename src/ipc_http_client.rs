use std::path::Path;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::AnyResult;
use crate::http_client::http_request;
use crate::types::Response;

/// Generic IPC HTTP client that works on both Unix and Windows platforms
///
/// This client is optimized for request-response patterns, not streaming.
/// For streaming functionality, use `IpcStreamClient` instead.
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

    /// 便捷方法：发送 GET 请求
    pub async fn get(&self, path: &str) -> AnyResult<Response> {
        self.request("GET", path, None).await
    }

    /// 便捷方法：发送 POST 请求
    pub async fn post(&self, path: &str, body: &serde_json::Value) -> AnyResult<Response> {
        self.request("POST", path, Some(body)).await
    }

    /// 便捷方法：发送 PUT 请求
    pub async fn put(&self, path: &str, body: &serde_json::Value) -> AnyResult<Response> {
        self.request("PUT", path, Some(body)).await
    }

    /// 便捷方法：发送 DELETE 请求
    pub async fn delete(&self, path: &str) -> AnyResult<Response> {
        self.request("DELETE", path, None).await
    }

    /// 便捷方法：发送 PATCH 请求
    pub async fn patch(&self, path: &str, body: &serde_json::Value) -> AnyResult<Response> {
        self.request("PATCH", path, Some(body)).await
    }
}
