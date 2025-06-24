use crate::errors::AnyResult;
use crate::types::Response;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::io::{AsyncRead, AsyncWrite};

pub async fn send_http_over_stream<S>(
    mut stream: S,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> AnyResult<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let body_bytes = if let Some(b) = body {
        Some(serde_json::to_vec(b)?)
    } else {
        None
    };

    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
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
    let mut content_length = None;
    let mut is_chunked = false;
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
            if key.eq_ignore_ascii_case("Content-Length") {
                content_length = value.parse::<usize>().ok();
            }
            if key.eq_ignore_ascii_case("Transfer-Encoding") && value.eq_ignore_ascii_case("chunked") {
                is_chunked = true;
            }
            headers_map.insert(key, value);
        }
    }
    let headers = serde_json::to_value(headers_map)?;

    let body_str = if is_chunked {
        let mut body = Vec::new();
        loop {
            let mut size_line = String::new();
            let n = reader.read_line(&mut size_line).await?;
            if n == 0 {
                break;
            }
            let size_line = size_line.trim();
            if size_line.is_empty() {
                continue;
            }
            let chunk_size = usize::from_str_radix(size_line, 16).unwrap_or(0);
            if chunk_size == 0 {
                // 读完最后一个 chunk 后还有一个空行
                let _ = reader.read_line(&mut String::new()).await;
                break;
            }
            let mut chunk = vec![0u8; chunk_size];
            reader.read_exact(&mut chunk).await?;
            body.extend_from_slice(&chunk);
            // 读掉 chunk 结尾的 \r\n
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf).await?;
        }
        String::from_utf8(body)?
    } else if let Some(len) = content_length {
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        String::from_utf8(body)?
    } else {
        let mut body = Vec::new();
        reader.read_to_end(&mut body).await?;
        String::from_utf8(body)?
    };

    Ok(Response {
        status,
        headers: headers,
        body: body_str,
    })
}
