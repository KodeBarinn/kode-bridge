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
            headers_map.insert(key, value);
        }
    }
    let headers = serde_json::to_value(headers_map)?;

    let body_str = if let Some(len) = content_length {
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        String::from_utf8(body)?
    } else {
        String::new()
    };

    Ok(Response {
        status,
        headers: headers,
        body: body_str,
    })
}
