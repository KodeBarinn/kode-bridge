# kode-bridge

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](http://www.apache.org/licenses/LICENSE-2.0)
[![Crates.io](https://img.shields.io/crates/v/kode-bridge.svg)](https://crates.io/crates/kode-bridge)

**中文 | [English](./README.md)**

**kode-bridge** 是一个现代化的 Rust 库，专为跨平台（macOS、Linux、Windows）IPC HTTP 通信而设计。通过统一的 API，你可以轻松地通过 Unix Domain Sockets 或 Windows Named Pipes 发送 HTTP 请求，就像使用普通的 HTTP 客户端一样简单。

## ✨ 特点

- **🌍 真正跨平台**：自动检测平台并使用最优的 IPC 方式
  - **Unix/Linux/macOS**: Unix Domain Sockets
  - **Windows**: Named Pipes
- **🚀 零配置使用**：统一的 `IpcHttpClient` API，无需平台特定代码
- **📦 自动序列化**：内置 JSON 请求与响应处理
- **⚡ 高性能**：针对不同平台优化的连接管理策略
- **🔧 易于集成**：基于 [interprocess](https://github.com/kotauskas/interprocess) 和 Tokio 异步运行时
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
use kode_bridge::IpcHttpClient;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 自动检测平台并使用适当的 IPC 路径
    #[cfg(unix)]
    let client = IpcHttpClient::new("/tmp/my_service.sock")?;
    
    #[cfg(windows)]
    let client = IpcHttpClient::new(r"\\.\pipe\my_service")?;
    
    // 发送 GET 请求
    let response = client.request("GET", "/api/version", None).await?;
    println!("Status: {}", response.status);
    println!("Response: {}", response.body);
    
    // 发送 POST 请求
    let data = json!({"user": "alice", "action": "login"});
    let response = client.request("POST", "/api/auth", Some(&data)).await?;
    println!("Auth result: {}", response.json()?);
    
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
│              IpcHttpClient              │
│       (Unified Cross-Platform API)      │
├─────────────────────────────────────────┤
│            http_client.rs               │
│        (HTTP Protocol Handler)          │
├─────────────────────────────────────────┤
│             interprocess                │
│       (Cross-Platform IPC Transport)    │
├─────────────────┬───────────────────────┤
│   Unix Sockets  │    Windows Pipes      │
│   (Unix/Linux)  │     (Windows)         │
└─────────────────┴───────────────────────┘
```

### 核心组件

- **`IpcHttpClient`**: 统一的客户端接口，自动适配不同平台
- **`http_client`**: 平台无关的 HTTP 协议处理，支持分块传输编码
- **智能平台检测**: 编译时自动选择最优的 IPC 实现

## 🎯 适用场景

- **本地服务通信**: 与 Clash、Mihomo、代理服务等本地进程通信
- **微服务架构**: 进程间高性能 HTTP 通信
- **系统集成**: 替代传统的 REST API 本地调用
- **性能敏感应用**: 需要低延迟本地通信的场景

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