# kode-bridge

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](http://www.apache.org/licenses/LICENSE-2.0)
[![Crates.io](https://img.shields.io/crates/v/kode-bridge.svg)](https://crates.io/crates/kode-bridge)

**中文 | [English](./README.md)**

**kode-bridge** 是一个现代化的 Rust 库，专为跨平台（macOS、Linux、Windows）IPC 通信而设计。它提供 HTTP 风格的请求/响应和实时流式传输能力，通过 Unix Domain Sockets 或 Windows Named Pipes 实现，具有类似 reqwest 的流畅 API。

## ✨ 特点

- **🌍 真正跨平台**：自动检测平台并使用最优的 IPC 方式
  - **Unix/Linux/macOS**: Unix Domain Sockets
  - **Windows**: Named Pipes
- **🚀 双客户端架构**：
  - **`IpcHttpClient`**: HTTP 风格的请求/响应，适用于 API 调用
  - **`IpcStreamClient`**: 实时流式传输，用于持续数据监控
- **💎 流畅 API**：受 reqwest 启发的方法链式调用，类型安全的 JSON 处理
- **📦 自动序列化**：内置 JSON 请求与响应处理
- **⚡ 高性能**：针对不同平台优化的连接管理策略
- **🔧 易于集成**：基于 [interprocess](https://github.com/kotauskas/interprocess) 和 Tokio 异步运行时
- **🔄 向后兼容**：旧版 API 方法与新流畅接口并存
- **📖 完整支持**：包含示例、基准测试和详细文档

## 🚀 快速开始

### 添加依赖

```toml
[dependencies]
kode-bridge = "0.1"
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
```

### 基本使用

```rust
use kode_bridge::{IpcHttpClient, IpcStreamClient};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 自动检测平台并使用适当的 IPC 路径
    #[cfg(unix)]
    let ipc_path = "/tmp/my_service.sock";
    #[cfg(windows)]
    let ipc_path = r"\\.\pipe\my_service";
    
    // HTTP 风格客户端，用于请求/响应
    let client = IpcHttpClient::new(ipc_path)?;
    
    // 🔥 全新流畅 API - 就像 reqwest 一样！
    let response = client
        .get("/api/version")
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    
    println!("状态: {}", response.status());
    println!("成功: {}", response.is_success());
    
    // 类型安全的 JSON 解析
    #[derive(serde::Deserialize)]
    struct ApiResponse {
        version: String,
        meta: bool,
    }
    
    let data: ApiResponse = response.json()?;
    println!("版本: {}", data.version);
    
    // 带 JSON 主体的 POST 请求
    let update_data = json!({"user": "alice", "action": "login"});
    let response = client
        .post("/api/auth")
        .json_body(&update_data)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    
    if response.is_success() {
        println!("认证成功！");
    }
    
    // 实时流式客户端
    let stream_client = IpcStreamClient::new(ipc_path)?;
    
    // 实时监控流量数据
    #[derive(serde::Deserialize, Debug)]
    struct TrafficData {
        up: u64,
        down: u64,
    }
    
    let traffic_data: Vec<TrafficData> = stream_client
        .get("/traffic")
        .timeout(Duration::from_secs(5))
        .json_results()
        .await?;
    
    println!("收集了 {} 个流量样本", traffic_data.len());
    
    Ok(())
}
```

### 高级用法

```rust
use kode_bridge::{IpcHttpClient, IpcStreamClient};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = IpcHttpClient::new("/tmp/service.sock")?;
    
    // 支持所有 HTTP 方法
    let response = client.put("/api/config")
        .json_body(&json!({"key": "value"}))
        .send()
        .await?;
    
    // 丰富的响应检查
    println!("状态: {}", response.status());
    println!("头部: {:?}", response.headers());
    println!("内容长度: {}", response.content_length());
    println!("是否客户端错误: {}", response.is_client_error());
    println!("是否服务端错误: {}", response.is_server_error());
    
    // 实时回调的流处理
    let stream_client = IpcStreamClient::new("/tmp/service.sock")?;
    
    stream_client
        .get("/events")
        .send()
        .await?
        .process_lines(|line| {
            println!("实时事件: {}", line);
            Ok(())
        })
        .await?;
    
    Ok(())
}
```

### 使用环境变量

创建 `.env` 文件：

```env
# Unix 系统
CUSTOM_SOCK=/tmp/my_app.sock

# Windows 系统（每个反斜杠都需双写进行转义）
CUSTOM_PIPE=\\\\.\\pipe\\\my_app
```

然后在代码中：

```rust
use dotenv::dotenv;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    
    #[cfg(unix)]
    let path = env::var("CUSTOM_SOCK").unwrap_or("/tmp/default.sock".to_string());
    
    #[cfg(windows)]
    let path = env::var("CUSTOM_PIPE").unwrap_or(r"\\.\pipe\default".to_string());
    
    let client = IpcHttpClient::new(&path)?;
    let response = client.request("GET", "/status", None).await?;
    
    Ok(())
}
```

## 📋 示例

运行内置示例：

```bash
# 基本请求示例
cargo run --example request

# 大数据请求示例
cargo run --example request_large

# 优雅 HTTP 客户端演示
cargo run --example elegant_http

# 优雅流式客户端演示
cargo run --example elegant_stream

# 双客户端对比
cargo run --example two_clients

# 实时流量监控
cargo run --example traffic

# 使用自定义 IPC 路径
CUSTOM_SOCK=/tmp/my.sock cargo run --example request  # Unix
CUSTOM_PIPE=\\\\.\\pipe\\my_pipe cargo run --example request  # Windows
```

## 🔥 性能基准测试

运行性能基准测试：

```bash
# 运行所有基准测试
cargo bench

# 查看基准测试报告
open target/criterion/report/index.html
```

基准测试会自动：
- 检测运行平台
- 使用适当的环境变量（`CUSTOM_SOCK` 或 `CUSTOM_PIPE`）
- 应用平台特定的性能优化策略

## 🏗️ 架构设计

```
┌─────────────────────────────────────────┐
│     IpcHttpClient    IpcStreamClient    │
│   (请求/响应模式)      (实时流式传输)     │
├─────────────────────────────────────────┤
│              流畅 API                   │
│   (HTTP 风格方法 & 方法链式调用)          │
├─────────────────────────────────────────┤
│            http_client.rs               │
│        (HTTP 协议处理器)                 │
├─────────────────────────────────────────┤
│             interprocess                │
│       (跨平台 IPC 传输层)                │
├─────────────────┬───────────────────────┤
│   Unix Sockets  │    Windows Pipes      │
│   (Unix/Linux)  │     (Windows)         │
└─────────────────┴───────────────────────┘
```

### 核心组件

- **`IpcHttpClient`**: HTTP 风格的请求/响应客户端，具有流畅 API
- **`IpcStreamClient`**: 实时流式客户端，用于持续数据监控
- **流畅 API**: 方法链式调用，支持 `get()`, `post()`, `timeout()`, `json_body()`, `send()` 等
- **`http_client`**: 平台无关的 HTTP 协议处理，支持分块传输编码
- **智能平台检测**: 编译时自动选择最优的 IPC 实现

### API 对比

| 功能 | 旧版 API | 新版流畅 API |
|------|----------|-------------|
| GET 请求 | `client.request("GET", "/path", None)` | `client.get("/path").send()` |
| POST 带 JSON | `client.request("POST", "/path", Some(&json))` | `client.post("/path").json_body(&json).send()` |
| 超时控制 | 不支持 | `client.get("/path").timeout(Duration::from_secs(5)).send()` |
| 响应状态 | `response.status` | `response.status()`, `response.is_success()` |
| JSON 解析 | `response.json()?` | `response.json::<T>()?` 具有类型推导 |
| 流式传输 | 不可用 | `stream_client.get("/events").json_results().await?` |

## 🎯 适用场景

- **本地服务通信**: 与 Clash、Mihomo、代理服务等本地进程通信
- **实时监控**: 流式传输流量数据、日志、指标和系统事件
- **微服务架构**: 进程间高性能 HTTP 通信
- **系统集成**: 用 IPC 替代传统的 REST API 本地调用
- **性能敏感应用**: 需要低延迟本地通信的场景
- **配置管理**: 动态配置更新，立即反馈

## 🛠️ 开发

### 构建项目

```bash
git clone https://github.com/KodeBarinn/kode-bridge.git
cd kode-bridge
cargo build
```

### 运行测试

```bash
cargo test
```

### 生成文档

```bash
cargo doc --open
```

## 📚 更多资源

- [Platform Guide](./PLATFORM_GUIDE.md) - 跨平台使用详细指南
- [Examples](./examples/) - 完整示例代码
- [Benchmarks](./benches/) - 性能基准测试

## 🤝 贡献

欢迎提交 Issue 和 Pull Request!

## 📄 License

This project is licensed under the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0).

See the [Licence](./Licence) file for details.