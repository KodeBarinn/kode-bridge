# kode-bridge

**kode-bridge** 是一个简洁的 Rust 库，旨在跨平台（macOS、Linux、Windows）通过进程间通信（IPC）通道发送 HTTP 请求。它基于 [`interprocess`](https://github.com/kotauskas/interprocess) 和标准库，封装了 HTTP over Unix Socket（未来支持 Windows 命名管道），让你像用 HTTP 客户端一样与本地服务通信。

## 特点

- **跨平台**：支持 macOS、Linux（Unix Socket），预留 Windows 命名管道接口。
- **简单易用**：统一的 `IpcHttpClient`，一行代码即可发起 HTTP 请求。
- **自动序列化**：支持 JSON 请求与响应自动序列化/反序列化。
- **高可扩展**：HTTP 解析与读写逻辑平台无关，易于扩展更多 IPC 方式。

## 快速开始

```rust
use kode_bridge::IpcHttpClient;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = IpcHttpClient::new("/tmp/some_socket_path.sock");
    let resp = client.request("POST", "/api", Some(&json!({"foo": "bar"}))).await?;
    println!("Status: {}", resp.status);
    println!("Headers: {:?}", resp.headers);
    println!("Body: {}", resp.body);
    println!("As JSON: {:?}", resp.json()?);
    Ok(())
}
```

## Benchmark

支持基于 [criterion](https://github.com/bheisler/criterion.rs) 的性能基准测试：


## 适用场景
与本地代理、服务端进程（如 Clash、Mihomo 等）进行高效、类型安全的 HTTP 通信
替代 RESTful HTTP 服务的本地 IPC 通道
需要跨平台 IPC HTTP 通信的 Rust 项目

## License

This project is licensed under the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0).

See the [Licence](./Licence) file for details.