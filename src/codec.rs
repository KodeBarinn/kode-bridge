use crate::errors::{KodeBridgeError, Result};
use crate::ipc_http_server::HttpResponse;
use bytes::{BufMut as _, Bytes, BytesMut};
use http::{HeaderMap, Method, Uri};
use httparse::{Request, Status};
use std::io::Write as _;
use tokio_util::codec::{Decoder, Encoder};

/// Codec for HTTP over IPC
pub struct HttpIpcCodec {
    max_header_size: usize,
    max_request_size: usize,
}

impl HttpIpcCodec {
    pub fn new(max_header_size: usize, max_request_size: usize) -> Self {
        Self {
            max_header_size,
            max_request_size,
        }
    }

    fn find_header_end(data: &[u8]) -> Option<usize> {
        data.windows(4).position(|window| window == b"\r\n\r\n")
    }
}

/// Parsed request parts
pub struct ParsedRequest {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Decoder for HttpIpcCodec {
    type Item = ParsedRequest;
    type Error = KodeBridgeError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        if src.is_empty() {
            return Ok(None);
        }

        // Try to find the end of the headers
        let header_end = match Self::find_header_end(src) {
            Some(end) => end,
            None => {
                if src.len() > self.max_header_size {
                    return Err(KodeBridgeError::validation("Header size exceeds maximum allowed"));
                }
                return Ok(None);
            }
        };

        // Parse headers to get Content-Length
        let headers_len = header_end + 4; // Include \r\n\r\n

        let mut headers = vec![httparse::EMPTY_HEADER; 64];
        let mut req = Request::new(&mut headers);

        // We only parse the header part to get Content-Length
        let status = req
            .parse(&src[..headers_len])
            .map_err(|e| KodeBridgeError::validation(format!("Failed to parse HTTP request: {}", e)))?;

        match status {
            Status::Complete(_body_start) => {
                // Find content-length
                let mut content_length = 0;
                for header in req.headers.iter() {
                    if header.name.eq_ignore_ascii_case("Content-Length") {
                        if let Ok(s) = std::str::from_utf8(header.value) {
                            if let Ok(len) = s.parse::<usize>() {
                                content_length = len;
                            }
                        }
                    }
                }

                let total_len = headers_len + content_length;

                if total_len > self.max_request_size {
                    return Err(KodeBridgeError::validation("Request size exceeds maximum allowed"));
                }

                if src.len() < total_len {
                    // Reserve space for the rest of the body
                    src.reserve(total_len - src.len());
                    return Ok(None);
                }

                // We have the full request. Split the buffer.
                let data = src.split_to(total_len);

                // Re-parse the full request to extract fields
                // We need new headers array because the previous one was for the slice
                let mut headers = vec![httparse::EMPTY_HEADER; 64];
                let mut req = Request::new(&mut headers);
                let status = req.parse(&data)?;
                #[allow(clippy::unwrap_used)]
                let body_start = status.unwrap();

                let method = req
                    .method
                    .ok_or_else(|| KodeBridgeError::validation("Missing HTTP method"))?;
                let method = Method::from_bytes(method.as_bytes())
                    .map_err(|e| KodeBridgeError::validation(format!("Invalid HTTP method: {}", e)))?;

                let path = req
                    .path
                    .ok_or_else(|| KodeBridgeError::validation("Missing HTTP path"))?;
                let uri = path
                    .parse::<Uri>()
                    .map_err(|e| KodeBridgeError::validation(format!("Invalid URI: {}", e)))?;

                let mut header_map = HeaderMap::new();
                for header in req.headers {
                    if let (Ok(name), Ok(value)) = (
                        header.name.parse::<http::header::HeaderName>(),
                        http::header::HeaderValue::from_bytes(header.value),
                    ) {
                        header_map.insert(name, value);
                    }
                }

                let bytes = data.freeze();
                let body = bytes.slice(body_start..);

                Ok(Some(ParsedRequest {
                    method,
                    uri,
                    headers: header_map,
                    body,
                }))
            }
            Status::Partial => Ok(None),
        }
    }
}

impl Encoder<HttpResponse> for HttpIpcCodec {
    type Error = KodeBridgeError;

    fn encode(&mut self, item: HttpResponse, dst: &mut BytesMut) -> Result<()> {
        dst.reserve(256 + item.body.len());

        let status = item.status;
        let reason = status.canonical_reason().unwrap_or("Unknown");

        let mut writer = dst.writer();
        write!(writer, "HTTP/1.1 {} {}\r\n", status.as_u16(), reason)
            .map_err(|e| KodeBridgeError::connection(format!("Failed to write response status: {}", e)))?;

        let mut has_content_length = false;
        for (key, value) in item.headers.iter() {
            write!(writer, "{}: ", key.as_str())
                .map_err(|e| KodeBridgeError::connection(format!("Failed to write header key: {}", e)))?;
            writer
                .write_all(value.as_bytes())
                .map_err(|e| KodeBridgeError::connection(format!("Failed to write header value: {}", e)))?;
            writer
                .write_all(b"\r\n")
                .map_err(|e| KodeBridgeError::connection(format!("Failed to write CRLF: {}", e)))?;

            if key.as_str().eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
        }

        if !has_content_length {
            write!(writer, "Content-Length: {}\r\n", item.body.len())
                .map_err(|e| KodeBridgeError::connection(format!("Failed to write Content-Length: {}", e)))?;
        }

        writer
            .write_all(b"\r\n")
            .map_err(|e| KodeBridgeError::connection(format!("Failed to write header end: {}", e)))?;
        writer
            .write_all(item.body.as_ref())
            .map_err(|e| KodeBridgeError::connection(format!("Failed to write body: {}", e)))?;
        writer
            .flush()
            .map_err(|e| KodeBridgeError::connection(format!("Failed to flush: {}", e)))?;

        Ok(())
    }
}
