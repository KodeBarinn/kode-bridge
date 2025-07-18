use crate::errors::AnyResult;
use crate::types::Response;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::Stream;
use tokio_stream::wrappers::LinesStream;

/// Streaming HTTP response that yields data as it arrives
pub struct StreamingResponse {
    pub status: u16,
    pub headers: serde_json::Value,
    pub stream: Pin<Box<dyn Stream<Item = Result<String, std::io::Error>> + Send>>,
}

pub async fn http_request_stream<S>(
    mut stream: S,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> AnyResult<StreamingResponse>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let body_bytes = if let Some(b) = body {
        Some(serde_json::to_vec(b)?)
    } else {
        None
    };

    #[cfg(unix)]
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
        method, path
    );
    #[cfg(windows)]
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n",
        method, path
    );

    if let Some(ref b) = body_bytes {
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    request.push_str("\r\n");

    if let Some(ref b) = body_bytes {
        let mut buf = Vec::with_capacity(request.len() + b.len());
        buf.extend_from_slice(request.as_bytes());
        buf.extend_from_slice(b);
        stream.write_all(&buf).await?;
    } else {
        stream.write_all(request.as_bytes()).await?;
    }
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).await?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    let mut headers_map = HashMap::new();
    let mut line = String::with_capacity(128);
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 || line == "\r\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(":") {
            let key = k.trim().to_string();
            let value = v.trim().to_string();
            headers_map.insert(key, value);
        }
    }
    let headers = serde_json::to_value(headers_map)?;

    // Create a streaming response
    let line_stream = LinesStream::new(reader.lines());
    let stream = Box::pin(
        line_stream
            .map(|result| result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
    );

    Ok(StreamingResponse {
        status,
        headers,
        stream,
    })
}

impl StreamingResponse {
    /// 优雅的 JSON 流处理 - 自动解析并过滤有效数据
    pub async fn json<T>(mut self, timeout: Duration) -> AnyResult<Vec<T>>
    where
        T: serde::de::DeserializeOwned + Send + 'static,
    {
        let mut results = Vec::new();
        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => {
                            if line.trim().is_empty() {
                                continue;
                            }
                            // 自动解析 JSON，失败则忽略
                            if let Ok(parsed) = serde_json::from_str::<T>(&line) {
                                results.push(parsed);
                            }
                        }
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                _ = &mut timeout_future => break,
            }
        }

        Ok(results)
    }

    /// 简化的流处理 - 处理 JSON 数据
    pub async fn process_json<F, T>(
        mut self,
        timeout: Duration,
        mut handler: F,
    ) -> AnyResult<Vec<T>>
    where
        F: FnMut(&str) -> Option<T>,
        T: Send + 'static,
    {
        let mut results = Vec::new();
        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => {
                            if let Some(parsed) = handler(&line) {
                                results.push(parsed);
                            }
                        }
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                _ = &mut timeout_future => break,
            }
        }

        Ok(results)
    }

    /// 实时处理流数据
    pub async fn process_realtime<F>(mut self, timeout: Duration, mut handler: F) -> AnyResult<()>
    where
        F: FnMut(&str) -> bool, // 返回 false 停止处理
    {
        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => {
                            if !handler(&line) {
                                break;
                            }
                        }
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                _ = &mut timeout_future => break,
            }
        }

        Ok(())
    }

    /// Collect all stream data into a regular Response
    pub async fn collect(mut self) -> AnyResult<Response> {
        let mut body_lines = Vec::new();

        while let Some(line_result) = self.stream.next().await {
            match line_result {
                Ok(line) => body_lines.push(line),
                Err(e) => return Err(Box::new(e)),
            }
        }

        Ok(Response {
            status: self.status,
            headers: self.headers,
            body: body_lines.join("\n"),
        })
    }

    /// Collect stream data with a timeout
    pub async fn collect_with_timeout(
        mut self,
        timeout: std::time::Duration,
    ) -> AnyResult<Response> {
        let mut body_lines = Vec::new();

        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => body_lines.push(line),
                        Some(Err(e)) => return Err(Box::new(e)),
                        None => break, // Stream ended
                    }
                }
                _ = &mut timeout_future => {
                    break; // Timeout reached
                }
            }
        }

        Ok(Response {
            status: self.status,
            headers: self.headers,
            body: body_lines.join("\n"),
        })
    }
}
