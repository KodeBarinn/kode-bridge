use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::{GenericFilePath, Name};
use interprocess::local_socket::{ToFsName, traits::tokio::Stream};

use crate::errors::{AnyError, AnyResult};
use crate::ipc_http::http_over_stream::send_http_over_stream;
use crate::types::Response;

pub struct UnixIpcHttpClient {
    socket_path: String,
}

impl UnixIpcHttpClient {
    pub fn set_path<S>(&mut self, path: S) -> Result<(), AnyError>
    where
        S: Into<String>,
    {
        self.socket_path = path.into();
        Ok(())
    }

    pub fn get_name(&self) -> Result<Name<'_>, AnyError> {
        Ok(self.socket_path.as_str().to_fs_name::<GenericFilePath>()?)
    }
}

impl UnixIpcHttpClient {
    pub fn new(socket_path: &str) -> Self {
        UnixIpcHttpClient {
            socket_path: socket_path.to_string(),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let name = self.get_name()?;
        let stream = LocalSocketStream::connect(name).await?;
        send_http_over_stream(stream, method, path, body).await
    }
}
