use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::AnyResult;
use crate::stream_client::{StreamingResponse, http_request_stream};

/// Specialized IPC streaming client for handling real-time data streams
pub struct IpcStreamClient {
    name: Name<'static>,
}

impl IpcStreamClient {
    /// Create a new IPC streaming client with the given socket/named pipe path
    ///
    /// # Arguments
    /// * `path` - The path to the socket (Unix) or named pipe (Windows)
    pub fn new<P: AsRef<Path>>(path: P) -> AnyResult<Self> {
        let name = path.as_ref().to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Self { name })
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

    /// 便捷方法：获取 JSON 流数据
    ///
    /// # Arguments
    /// * `path` - Request path
    /// * `timeout` - Timeout duration for collecting data
    pub async fn get_json_stream<T>(&self, path: &str, timeout: Duration) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send + 'static,
    {
        let response = self.request_stream("GET", path, None).await?;
        response.json(timeout).await
    }

    /// 便捷方法：实时处理流数据
    ///
    /// # Arguments
    /// * `path` - Request path
    /// * `timeout` - Timeout duration for processing
    /// * `handler` - Closure to handle each line of data
    pub async fn process_stream<F>(
        &self,
        path: &str,
        timeout: Duration,
        handler: F,
    ) -> AnyResult<()>
    where
        F: FnMut(&str) -> bool, // 返回 false 停止处理
    {
        let response = self.request_stream("GET", path, None).await?;
        response.process_realtime(timeout, handler).await
    }

    /// 便捷方法：监控流量数据
    pub async fn monitor_traffic(&self, timeout: Duration) -> AnyResult<Vec<TrafficData>> {
        self.get_json_stream("/traffic", timeout).await
    }

    /// 便捷方法：监控连接状态
    pub async fn monitor_connections(&self, timeout: Duration) -> AnyResult<Vec<ConnectionData>> {
        self.get_json_stream("/connections", timeout).await
    }
}

/// 流量数据结构
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TrafficData {
    pub up: u64,
    pub down: u64,
}

/// 连接数据结构
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ConnectionData {
    pub id: String,
    pub metadata: serde_json::Value,
    pub upload: u64,
    pub download: u64,
    pub start: String,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
}
