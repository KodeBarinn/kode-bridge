use crate::errors::{KodeBridgeError, Result};
use bytes::Bytes;
use futures::stream::StreamExt;
use http::{header, HeaderMap, StatusCode};
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_stream::Stream;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{debug, trace, warn};

pin_project! {
    /// Streaming HTTP response that yields data as it arrives
    pub struct StreamingResponse {
        pub status: StatusCode,
        pub headers: HeaderMap,
        #[pin]
        pub stream: Pin<Box<dyn Stream<Item = std::result::Result<String, std::io::Error>> + Send>>,
    }
}

impl StreamingResponse {
    pub fn new(
        status: StatusCode,
        headers: HeaderMap,
        stream: Pin<Box<dyn Stream<Item = std::result::Result<String, std::io::Error>> + Send>>,
    ) -> Self {
        Self {
            status,
            headers,
            stream,
        }
    }

    /// Get HTTP status code
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get status code as u16
    pub fn status_code(&self) -> u16 {
        self.status.as_u16()
    }

    /// Get response headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Check if response indicates success (2xx status)
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Check if response indicates client error (4xx status)
    pub fn is_client_error(&self) -> bool {
        self.status.is_client_error()
    }

    /// Check if response indicates server error (5xx status)
    pub fn is_server_error(&self) -> bool {
        self.status.is_server_error()
    }

    /// Get content length from headers
    pub fn content_length(&self) -> Option<u64> {
        self.headers
            .get(header::CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse()
            .ok()
    }

    /// Get content type from headers
    pub fn content_type(&self) -> Option<&str> {
        self.headers.get(header::CONTENT_TYPE)?.to_str().ok()
    }

    /// Elegant JSON stream processing - automatically parse and filter valid data
    pub async fn json<T>(mut self, timeout: Duration) -> Result<Vec<T>>
    where
        T: DeserializeOwned + Send,
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
                            // Auto-parse JSON, ignore failures for robustness
                            if let Ok(parsed) = serde_json::from_str::<T>(&line) {
                                results.push(parsed);
                            } else {
                                trace!("Failed to parse JSON line: {}", line);
                            }
                        }
                        Some(Err(e)) => {
                            warn!("Stream error: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
                _ = &mut timeout_future => {
                    debug!("Stream timeout reached after {}ms", timeout.as_millis());
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Process stream with custom JSON handler
    pub async fn process_json<F, T>(mut self, timeout: Duration, mut handler: F) -> Result<Vec<T>>
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
                        Some(Err(e)) => {
                            warn!("Stream error: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
                _ = &mut timeout_future => break,
            }
        }

        Ok(results)
    }

    /// Process stream data in real-time with error handling
    pub async fn process_lines<F>(mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(&str) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
    {
        while let Some(line_result) = self.stream.next().await {
            match line_result {
                Ok(line) => {
                    if let Err(e) = handler(&line) {
                        warn!("Handler error: {}", e);
                        return Err(KodeBridgeError::custom(format!("Handler error: {}", e)));
                    }
                }
                Err(e) => {
                    warn!("Stream error: {}", e);
                    return Err(KodeBridgeError::from(e));
                }
            }
        }
        Ok(())
    }

    /// Process stream with timeout and error handling - optimized for better performance
    pub async fn process_lines_with_timeout<F>(
        mut self,
        timeout: Duration,
        mut handler: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> std::result::Result<bool, Box<dyn std::error::Error + Send + Sync>>, // Return false to stop
    {
        // 使用更短的超时避免长时间的waker等待
        let optimized_timeout = std::cmp::min(timeout, Duration::from_secs(5));
        let timeout_future = tokio::time::sleep(optimized_timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => {
                            match handler(&line) {
                                Ok(continue_processing) => {
                                    if !continue_processing {
                                        break;
                                    }
                                    // 重置超时计时器以避免不必要的超时
                                    timeout_future.as_mut().reset(tokio::time::Instant::now() + optimized_timeout);
                                }
                                Err(e) => {
                                    warn!("Handler error: {}", e);
                                    return Err(KodeBridgeError::custom(format!("Handler error: {}", e)));
                                }
                            }
                        }
                        Some(Err(e)) => {
                            warn!("Stream error: {}", e);
                            return Err(KodeBridgeError::from(e));
                        }
                        None => break,
                    }
                }
                _ = &mut timeout_future => {
                    debug!("Processing timeout reached ({:?})", optimized_timeout);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Collect all stream data into a string
    pub async fn collect_text(mut self) -> Result<String> {
        let mut body_lines = Vec::new();

        while let Some(line_result) = self.stream.next().await {
            match line_result {
                Ok(line) => body_lines.push(line),
                Err(e) => return Err(KodeBridgeError::from(e)),
            }
        }

        Ok(body_lines.join("\n"))
    }

    /// Collect stream data with a timeout - optimized for better performance
    pub async fn collect_text_with_timeout(mut self, timeout: Duration) -> Result<String> {
        let mut body_lines = Vec::new();

        // 限制最大超时时间避免长时间waker等待
        let optimized_timeout = std::cmp::min(timeout, Duration::from_secs(30));
        let timeout_future = tokio::time::sleep(optimized_timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                line_result = self.stream.next() => {
                    match line_result {
                        Some(Ok(line)) => {
                            body_lines.push(line);
                            // 收到数据后重置超时，避免不必要的超时
                            timeout_future.as_mut().reset(tokio::time::Instant::now() + optimized_timeout);
                        }
                        Some(Err(e)) => return Err(KodeBridgeError::from(e)),
                        None => break, // Stream ended
                    }
                }
                _ = &mut timeout_future => {
                    debug!("Collection timeout reached");
                    break; // Timeout reached
                }
            }
        }

        Ok(body_lines.join("\n"))
    }

    /// Convert to legacy format for compatibility
    pub fn status_u16(&self) -> u16 {
        self.status.as_u16()
    }

    /// Get headers as JSON value for compatibility
    pub fn headers_json(&self) -> Value {
        let headers_map: HashMap<String, String> = self
            .headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        serde_json::to_value(headers_map).unwrap_or(Value::Null)
    }
}

impl Stream for StreamingResponse {
    type Item = std::result::Result<String, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.stream.poll_next(cx)
    }
}

/// Parse HTTP response headers and create streaming response
pub async fn parse_streaming_response<S>(stream: S) -> Result<StreamingResponse>
where
    S: AsyncRead + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();

    // Read until we have the complete headers
    let mut headers_end = None;
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 {
            return Err(KodeBridgeError::protocol("Unexpected end of stream"));
        }

        buffer.extend_from_slice(&line);

        // Check for end of headers (\r\n\r\n)
        if buffer.len() >= 4 {
            for i in 0..buffer.len() - 3 {
                if &buffer[i..i + 4] == b"\r\n\r\n" {
                    headers_end = Some(i + 4);
                    break;
                }
            }
        }

        if headers_end.is_some() {
            break;
        }
    }

    let headers_end = headers_end
        .ok_or_else(|| KodeBridgeError::protocol("Could not find end of HTTP headers"))?;

    // Parse the headers using httparse
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut headers);

    let status = match response.parse(&buffer[..headers_end])? {
        httparse::Status::Complete(_) => response
            .code
            .ok_or_else(|| KodeBridgeError::protocol("HTTP response missing status code"))?,
        httparse::Status::Partial => {
            return Err(KodeBridgeError::protocol("Incomplete HTTP response"));
        }
    };

    // Build HeaderMap
    let mut header_map = HeaderMap::new();
    for header in response.headers {
        let name =
            http::HeaderName::from_str(header.name).map_err(|e| KodeBridgeError::Http(e.into()))?;
        let value = http::HeaderValue::from_bytes(header.value)
            .map_err(|e| KodeBridgeError::Http(e.into()))?;
        header_map.insert(name, value);
    }

    // Create line stream from the remaining reader
    let framed = FramedRead::new(reader, LinesCodec::new());
    let line_stream = framed.map(|result| result.map_err(std::io::Error::other));

    Ok(StreamingResponse::new(
        StatusCode::from_u16(status)?,
        header_map,
        Box::pin(line_stream),
    ))
}

/// Send HTTP request and get streaming response
pub async fn send_streaming_request<S>(mut stream: S, request: Bytes) -> Result<StreamingResponse>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Send request
    stream.write_all(&request).await?;
    stream.flush().await?;

    trace!("Sent HTTP streaming request ({} bytes)", request.len());

    // Parse response
    let response = parse_streaming_response(stream).await?;

    debug!(
        "Received HTTP streaming response: {} {}",
        response.status(),
        response.content_length().unwrap_or(0)
    );

    Ok(response)
}
