use crate::errors::{KodeBridgeError, Result};
use bytes::{Bytes, BytesMut};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Version, header};
use serde::{Serialize, de::DeserializeOwned};
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

    /// Add a header
    #[allow(dead_code)]
    pub fn header<K, V>(mut self, key: K, value: V) -> Result<Self>
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        let key = HeaderName::try_from(key).map_err(|e| KodeBridgeError::Http(e.into()))?;
        let value = HeaderValue::try_from(value).map_err(|e| KodeBridgeError::Http(e.into()))?;
        self.headers.insert(key, value);
        Ok(self)
    }

    /// Set JSON body
    pub fn json<T>(mut self, body: &T) -> Result<Self>
    where
        T: Serialize,
    {
        let json_bytes = serde_json::to_vec(body)?;
        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        self.headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&json_bytes.len().to_string())
                .map_err(|e| KodeBridgeError::Http(e.into()))?,
        );
        self.body = Some(Bytes::from(json_bytes));
        Ok(self)
    }

    /// Set text body
    #[allow(dead_code)]
    pub fn text<T: AsRef<str>>(mut self, body: T) -> Result<Self> {
        let text_bytes = body.as_ref().as_bytes().to_vec();
        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        self.headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&text_bytes.len().to_string())
                .map_err(|e| KodeBridgeError::Http(e.into()))?,
        );
        self.body = Some(Bytes::from(text_bytes));
        Ok(self)
    }

    /// Set raw bytes body
    #[allow(dead_code)]
    pub fn bytes(mut self, body: Bytes) -> Result<Self> {
        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        self.headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&body.len().to_string())
                .map_err(|e| KodeBridgeError::Http(e.into()))?,
        );
        self.body = Some(body);
        Ok(self)
    }

    /// Build the HTTP request as bytes
    pub fn build(self) -> Result<Bytes> {
        let mut request = BytesMut::new();

        // Request line
        let request_line = format!("{} {} HTTP/1.1\r\n", self.method, self.uri);
        request.extend_from_slice(request_line.as_bytes());

        // Headers
        for (key, value) in &self.headers {
            let header_line = format!("{}: {}\r\n", key, value.to_str().unwrap_or(""));
            request.extend_from_slice(header_line.as_bytes());
        }

        // End of headers
        request.extend_from_slice(b"\r\n");

        // Body
        if let Some(body) = self.body {
            request.extend_from_slice(&body);
        }

        Ok(request.freeze())
    }
}

/// Parse HTTP response from a stream
pub async fn parse_response<S>(stream: S) -> Result<Response>
where
    S: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut buffer = String::new();

    // Optimization: read the status line at once
    reader.read_line(&mut buffer).await?;

    // Continue reading headers
    let mut headers_buffer = Vec::new();
    headers_buffer.extend_from_slice(buffer.as_bytes());

    // Read remaining headers until \r\n\r\n is found
    let mut line_buffer = String::new();
    loop {
        line_buffer.clear();
        let n = reader.read_line(&mut line_buffer).await?;
        if n == 0 {
            return Err(KodeBridgeError::protocol("Unexpected end of stream"));
        }

        headers_buffer.extend_from_slice(line_buffer.as_bytes());

        // Check for empty line (just \r\n)
        if line_buffer == "\r\n" {
            break;
        }

        // Prevent headers from being too large
        if headers_buffer.len() > 16384 {
            return Err(KodeBridgeError::protocol("HTTP headers too large"));
        }
    }

    // Parse the headers
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut headers);

    let status = match response.parse(&headers_buffer)? {
        httparse::Status::Complete(_) => response.code.unwrap(),
        httparse::Status::Partial => {
            return Err(KodeBridgeError::protocol("Incomplete HTTP response"));
        }
    };

    // Build HeaderMap
    let mut header_map = HeaderMap::new();
    for header in response.headers {
        let name =
            HeaderName::from_str(header.name).map_err(|e| KodeBridgeError::Http(e.into()))?;
        let value =
            HeaderValue::from_bytes(header.value).map_err(|e| KodeBridgeError::Http(e.into()))?;
        header_map.insert(name, value);
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
            // Empty response, return directly
            Bytes::new()
        } else {
            read_fixed_body(&mut reader, len).await?
        }
    } else {
        // For responses without Content-Length, set a short timeout
        // This usually means the server may not send more data
        match tokio::time::timeout(Duration::from_millis(100), async {
            let mut body = Vec::new();
            reader.read_to_end(&mut body).await?;
            Ok::<_, std::io::Error>(Bytes::from(body))
        })
        .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(e)) => return Err(KodeBridgeError::from(e)),
            Err(_) => {
                // Timeout, assume no more data
                Bytes::new()
            }
        }
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
    let mut body = BytesMut::new();

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

        // Read chunk data
        let mut chunk = vec![0u8; chunk_size];
        reader.read_exact(&mut chunk).await?;
        body.extend_from_slice(&chunk);

        // Read trailing CRLF
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).await?;
    }

    Ok(body.freeze())
}

async fn read_fixed_body<R>(reader: &mut BufReader<R>, len: usize) -> Result<Bytes>
where
    R: AsyncRead + Unpin,
{
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Bytes::from(body))
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

/// Convenience function for making HTTP requests
#[allow(dead_code)]
pub async fn http_request<S>(
    stream: S,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let method = Method::from_str(method).map_err(|e| KodeBridgeError::Http(e.into()))?;

    let mut builder = RequestBuilder::new(method, path.to_string());

    if let Some(json_body) = body {
        builder = builder.json(json_body)?;
    }

    let request = builder.build()?;
    send_request(stream, request).await
}
