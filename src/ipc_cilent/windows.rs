pub struct WindowsIpcHttpClient {
    socket_path: String,
}

impl WindowsIpcHttpClient {
    pub fn new(socket_path: &str) -> Self {
        WindowsIpcHttpClient {
            socket_path: socket_path.to_string(),
        }
    }
}

#[async_trait]
impl SharedIpcHttpClient for WindowsIpcHttpClient {
    async fn new(socket_path: &str) -> Self {
        WindowsIpcHttpClient::new(socket_path)
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> AnyResult<Response> {
        let name = self.get_name()?;
        // TODO Replace to Windows specific socket connection namedpipe
        let stream = LocalSocketStream::connect(name).await?;
        let body_bytes = if let Some(b) = body {
            Some(serde_json::to_vec(b)?)
        } else {
            None
        };
        send_http_over_stream(
            stream,
            method,
            path,
            body_bytes.as_deref(),
            Some("application/json"),
        )
        .await
    }
}
