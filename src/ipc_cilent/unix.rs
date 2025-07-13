use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::{AnyError, AnyResult};
use crate::ipc_http::http_over_stream::send_http_over_stream;
use crate::types::Response;

pub struct UnixIpcHttpClient {
    socket_path: String,
}

impl UnixIpcHttpClient {
    pub fn new<S: Into<String>>(socket_path: S) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn set_path<S: Into<String>>(&mut self, path: S) {
        self.socket_path = path.into();
    }

    pub fn to_name(&self) -> Result<Name<'_>, AnyError> {
        self.socket_path.as_str().to_fs_name::<GenericFilePath>()
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let stream = LocalSocketStream::connect(self.to_name()?).await?;
        send_http_over_stream(stream, method, path, body).await
    }
}