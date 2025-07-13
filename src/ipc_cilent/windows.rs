use interprocess::os::windows::named_pipe::tokio::PipeStream;

use crate::errors::AnyResult;
use crate::ipc_http::http_over_stream::send_http_over_stream;
use crate::types::Response;

pub struct WindowsIpcHttpClient {
    named_path: String,
}

impl WindowsIpcHttpClient {
    pub fn new<S>(named_path: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            named_path: named_path.into(),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let stream = PipeStream::connect_by_path(self.named_path.as_str()).await?;
        send_http_over_stream(stream, method, path, body).await
    }
}
