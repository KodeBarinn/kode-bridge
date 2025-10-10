use crate::buffer_pool::global_pools;
use crate::errors::{KodeBridgeError, Result};
use crate::parser_cache::global_parser_cache;
use bytes::Bytes;
use http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Version};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, trace};

/// Enhanced HTTP response with rich functionality
#[derive(Debug, Clone)]
pub struct Response {
    status: StatusCode,
    version: Version,
    headers: HeaderMap,
    body: Bytes,
}

impl Response {
    pub fn new(status: StatusCode, version: Version, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            status,
            version,
            headers,
            body,
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

    /// Get HTTP version
    pub fn version(&self) -> Version {
        self.version
    }

    /// Get response headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get response body as bytes
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Get response body as string
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.to_vec()).map_err(KodeBridgeError::from)
    }

    /// Parse response body as JSON
    pub fn json<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).map_err(KodeBridgeError::from)
    }

    /// Parse response body as generic JSON value
    pub fn json_value(&self) -> Result<Value> {
        serde_json::from_slice(&self.body).map_err(KodeBridgeError::from)
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

    /// Check if response indicates redirection (3xx status)
    pub fn is_redirection(&self) -> bool {
        self.status.is_redirection()
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

    /// Convert to legacy Response format for backward compatibility
    pub fn to_legacy(&self) -> crate::response::LegacyResponse {
        let headers_map: HashMap<String, String> = self
            .headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        crate::response::LegacyResponse {
            status: self.status.as_u16(),
            headers: serde_json::to_value(headers_map).unwrap_or(Value::Null),
            body: String::from_utf8_lossy(&self.body).to_string(),
        }
    }
}

/// HTTP request builder with fluent interface
#[derive(Debug)]
pub struct RequestBuilder {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Option<Bytes>,
}

impl RequestBuilder {
    pub fn new(method: Method, uri: String) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost"));
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("kode-bridge/0.1"),
        );

        Self {
            method,
            uri,
            headers,
            body: None,
        }
    }

    /// Add a custom header
    pub fn header(mut self, key: &str, value: &str) -> Self {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            self.headers.insert(name, val);
        }
        self
    }

    /// Set JSON body with optimized serialization
    pub fn json<T>(mut self, body: &T) -> Result<Self>
    where
        T: Serialize,
    {
        // 使用全局缓冲池来减少内存分配
        let mut buffer = global_pools().get_medium();

        // 直接序列化到缓冲区，避免中间Vec分配
        serde_json::to_writer(buffer.as_mut_vec(), body)?;

        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        // 避免字符串分配，直接使用itoa
        let content_length = buffer.len().to_string();
        self.headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&content_length).map_err(|e| KodeBridgeError::Http(e.into()))?,
        );

        // 零拷贝转换到Bytes
        self.body = Some(Bytes::copy_from_slice(&buffer));
        Ok(self)
    }

    /// Build the HTTP request as bytes with optimized allocation
    pub fn build(self) -> Result<Bytes> {
        // 预估请求大小以选择合适的缓冲区
        let estimated_size = 200 + // 基础头部大小
            self.body.as_ref().map(|b| b.len()).unwrap_or(0);

        let mut request = if estimated_size > 8192 {
            global_pools().get_large()
        } else {
            global_pools().get_medium()
        };

        // Request line - 使用更高效的写入方式
        request.extend_from_slice(self.method.as_str().as_bytes());
        request.extend_from_slice(b" ");
        request.extend_from_slice(self.uri.as_bytes());
        request.extend_from_slice(b" HTTP/1.1\r\n");

        // Headers - 批量写入减少系统调用
        let mut headers_buffer = Vec::with_capacity(512);
        for (key, value) in &self.headers {
            headers_buffer.extend_from_slice(key.as_str().as_bytes());
            headers_buffer.extend_from_slice(b": ");
            headers_buffer.extend_from_slice(value.as_bytes());
            headers_buffer.extend_from_slice(b"\r\n");
        }
        request.extend_from_slice(&headers_buffer);

        // End of headers
        request.extend_from_slice(b"\r\n");

        // Body
        if let Some(body) = self.body {
            request.extend_from_slice(&body);
        }

        Ok(Bytes::copy_from_slice(&request))
    }
}

/// Parse HTTP response from a stream with optimized parsing
pub async fn parse_response<S>(stream: S) -> Result<Response>
where
    S: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);

    // 使用预分配的缓冲区避免多次分配
    let mut headers_buffer = global_pools().get_medium();

    // 先读取状态行
    let mut status_line = String::new();
    reader.read_line(&mut status_line).await?;
    headers_buffer.extend_from_slice(status_line.as_bytes());

    // 读取HTTP头部直到遇到空行
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break;
        }

        // 检查是否是头部结束（空行）
        if line == "\r\n" || line == "\n" {
            break;
        }

        // 将头部行添加到缓冲区
        headers_buffer.extend_from_slice(line.as_bytes());
    }

    // 添加最后的 \r\n 来标记头部结束
    headers_buffer.extend_from_slice(b"\r\n");

    // Use cached parser for better performance
    let mut parser = global_parser_cache().get();
    let (status, parsed_headers) = parser
        .parse_response(&headers_buffer)
        .map_err(|e| match e {
            httparse::Error::TooManyHeaders => {
                KodeBridgeError::protocol("Too many HTTP headers in response (limit: 64)")
            }
            _ => KodeBridgeError::protocol(format!("Failed to parse HTTP response: {:?}", e)),
        })?;

    // Build HeaderMap
    let mut header_map = HeaderMap::new();
    for (name, value) in parsed_headers {
        let header_name =
            HeaderName::from_str(&name).map_err(|e| KodeBridgeError::Http(e.into()))?;
        let header_value =
            HeaderValue::from_str(&value).map_err(|e| KodeBridgeError::Http(e.into()))?;
        header_map.insert(header_name, header_value);
    }

    // Determine body length
    let content_length = header_map
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok());

    let is_chunked = header_map
        .get(header::TRANSFER_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("chunked"))
        .unwrap_or(false);

    // Read body with optimized handling for empty responses (common in PUT/POST)
    let body = if is_chunked {
        read_chunked_body(&mut reader).await?
    } else if let Some(len) = content_length {
        if len == 0 {
            // Empty response, return directly - optimize for PUT responses
            Bytes::new()
        } else if len > 10 * 1024 * 1024 {
            // Use streaming for very large responses (>10MB)
            return Err(KodeBridgeError::protocol(
                "Response body too large for memory",
            ));
        } else {
            read_fixed_body(&mut reader, len).await?
        }
    } else {
        // For responses without Content-Length, use adaptive reading with shorter timeouts for PUT
        read_until_end_adaptive(&mut reader).await?
    };

    Ok(Response::new(
        StatusCode::from_u16(status)?,
        Version::HTTP_11,
        header_map,
        body,
    ))
}

async fn read_chunked_body<R>(reader: &mut BufReader<R>) -> Result<Bytes>
where
    R: AsyncRead + Unpin,
{
    let mut body_buffer = global_pools().get_large(); // 开始就使用大缓冲区

    loop {
        // Read chunk size line
        let mut size_line = String::new();
        reader.read_line(&mut size_line).await?;

        let size_line = size_line.trim();
        if size_line.is_empty() {
            continue;
        }

        // Parse chunk size (hex)
        let chunk_size = usize::from_str_radix(size_line, 16)
            .map_err(|_| KodeBridgeError::protocol("Invalid chunk size"))?;

        if chunk_size == 0 {
            // Last chunk, read final CRLF
            let mut final_line = String::new();
            reader.read_line(&mut final_line).await?;
            break;
        }

        // 对大块数据使用更大的缓冲区
        if chunk_size > 65536 && body_buffer.capacity() < 131072 {
            // 如果遇到大块，升级到更大的缓冲区
            let mut larger_buffer = global_pools().get_extra_large();
            larger_buffer.extend_from_slice(&body_buffer);
            body_buffer = larger_buffer;
        }

        // Read chunk data
        let mut chunk = vec![0u8; chunk_size];
        reader.read_exact(&mut chunk).await?;
        body_buffer.extend_from_slice(&chunk);

        // Read trailing CRLF
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).await?;

        // 安全限制 - 针对PUT请求增加到20MB
        if body_buffer.len() > 20 * 1024 * 1024 {
            return Err(KodeBridgeError::protocol("Chunked response body too large"));
        }
    }

    Ok(Bytes::copy_from_slice(body_buffer.as_slice()))
}

async fn read_fixed_body<R>(reader: &mut BufReader<R>, len: usize) -> Result<Bytes>
where
    R: AsyncRead + Unpin,
{
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Bytes::from(body))
}

async fn read_until_end_adaptive<R>(reader: &mut BufReader<R>) -> Result<Bytes>
where
    R: AsyncRead + Unpin,
{
    let mut body_buffer = global_pools().get_medium(); // 从中等缓冲区开始
    let mut read_buffer = [0u8; 8192]; // 增加读缓冲区大小
    let mut consecutive_empty_reads = 0;

    loop {
        // 使用更短的超时时间，更快地检测结束
        let timeout_duration = Duration::from_millis(50 + (consecutive_empty_reads * 25));

        match tokio::time::timeout(timeout_duration, reader.read(&mut read_buffer)).await {
            Ok(Ok(0)) => {
                // EOF reached
                break;
            }
            Ok(Ok(n)) => {
                // 如果数据量大，自动升级缓冲区
                if body_buffer.len() + n > body_buffer.capacity() * 3 / 4
                    && body_buffer.capacity() < 131072
                {
                    let mut larger_buffer = if body_buffer.capacity() < 16384 {
                        global_pools().get_large()
                    } else {
                        global_pools().get_extra_large()
                    };
                    larger_buffer.extend_from_slice(&body_buffer);
                    body_buffer = larger_buffer;
                }

                body_buffer.extend_from_slice(&read_buffer[..n]);
                consecutive_empty_reads = 0;
            }
            Ok(Err(e)) => {
                return Err(KodeBridgeError::from(e));
            }
            Err(_) => {
                // Timeout occurred
                consecutive_empty_reads += 1;
                if consecutive_empty_reads >= 2 {
                    // 减少到2次连续超时就结束，提高响应速度
                    break;
                }
                continue;
            }
        }

        // Safety limit to prevent unbounded memory usage - 增加到20MB
        if body_buffer.len() > 20 * 1024 * 1024 {
            return Err(KodeBridgeError::protocol("Response body too large"));
        }
    }

    // Convert pooled buffer to Bytes for return
    Ok(Bytes::copy_from_slice(body_buffer.as_slice()))
}

/// Send HTTP request and parse response
pub async fn send_request<S>(mut stream: S, request: Bytes) -> Result<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Send request
    stream.write_all(&request).await?;
    stream.flush().await?;

    trace!("Sent HTTP request ({} bytes)", request.len());

    // Parse response
    let response = parse_response(stream).await?;

    debug!(
        "Received HTTP response: {} {}",
        response.status(),
        response.content_length().unwrap_or(0)
    );

    Ok(response)
}
