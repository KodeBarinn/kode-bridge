use interprocess::local_socket::tokio::prelude::LocalSocketStream;
use interprocess::local_socket::traits::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use crate::errors::AnyResult;
use crate::ipc_http::http_over_stream::send_http_over_stream;
use crate::types::Response;

pub struct UnixIpcHttpClient {
    name: Name<'static>,
}

impl UnixIpcHttpClient {
    pub fn new<S>(socket_path: S) -> AnyResult<Self>
    where
        S: Into<String>,
    {
        let name = socket_path
            .into()
            .to_fs_name::<GenericFilePath>()?
            .into_owned();
        Ok(Self { name })
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let stream = LocalSocketStream::connect(self.name.clone()).await?;
        send_http_over_stream(stream, method, path, body).await
    }
}
