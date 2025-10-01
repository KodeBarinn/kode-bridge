//! Basic HTTP IPC server example
//!
//! This example demonstrates how to create a simple HTTP-style IPC server
//! that can handle various types of requests with routing and JSON responses.

use kode_bridge::ipc_http_server::{HttpResponse, IpcHttpServer, Router};
use kode_bridge::{Result, ServerConfig};
use serde_json::json;
use std::env;
use std::time::Duration;
use tokio::signal;
use tracing::info;

#[derive(serde::Serialize, serde::Deserialize)]
struct ApiVersion {
    name: String,
    version: String,
    build: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UserData {
    id: u64,
    name: String,
    email: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CreateUserRequest {
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("🚀 HTTP IPC Server Example");
    println!("==========================");

    // Get IPC path from environment or use default
    #[cfg(unix)]
    let ipc_path = env::var("CUSTOM_SOCK").unwrap_or_else(|_| "/tmp/http_server.sock".to_string());
    #[cfg(windows)]
    let ipc_path = env::var("CUSTOM_PIPE").unwrap_or_else(|_| r"\\.\pipe\http_server".to_string());

    println!("📡 Server will listen on: {}", ipc_path);

    // Configure server
    let config = ServerConfig {
        max_connections: 50,
        read_timeout: Duration::from_secs(30),
        write_timeout: Duration::from_secs(30),
        max_request_size: 1024 * 1024, // 1MB
        enable_logging: true,
        shutdown_timeout: Duration::from_secs(5),
        ..Default::default()
    };

    // Create router with various endpoints
    let router = Router::new()
        // GET /version - Return API version information
        .get("/version", |_ctx| async move {
            let version = ApiVersion {
                name: "HTTP IPC Server Example".to_string(),
                version: "1.0.0".to_string(),
                build: "example-build".to_string(),
            };
            let headers = _ctx.headers;
            println!("Headers received: {:?}", headers);
            HttpResponse::json(&version)
        })
        // GET /health - Health check endpoint
        .get("/health", |_ctx| async move {
            HttpResponse::json(&json!({
                "status": "healthy",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "uptime": "available"
            }))
        })
        // GET /users - List users (mock data)
        .get("/users", |_ctx| async move {
            let users = vec![
                UserData {
                    id: 1,
                    name: "Alice".to_string(),
                    email: "alice@example.com".to_string(),
                },
                UserData {
                    id: 2,
                    name: "Bob".to_string(),
                    email: "bob@example.com".to_string(),
                },
            ];
            HttpResponse::json(&users)
        })
        // GET /users/{id} - Get user by ID (simplified routing)
        .get("/user/1", |_ctx| async move {
            let user = UserData {
                id: 1,
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
            };
            HttpResponse::json(&user)
        })
        // POST /users - Create a new user
        .post("/users", |ctx| async move {
            match ctx.json::<CreateUserRequest>() {
                Ok(request) => {
                    // Validate request
                    if request.name.is_empty() || request.email.is_empty() {
                        return Ok(HttpResponse::builder()
                            .status(http::StatusCode::BAD_REQUEST)
                            .json(&json!({
                                "error": "Name and email are required"
                            }))?
                            .build());
                    }

                    // Create user (mock)
                    let user = UserData {
                        id: 999, // Mock ID
                        name: request.name,
                        email: request.email,
                    };

                    Ok(HttpResponse::builder()
                        .status(http::StatusCode::CREATED)
                        .json(&user)?
                        .build())
                }
                Err(e) => Ok(HttpResponse::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .json(&json!({
                        "error": format!("Invalid JSON: {}", e)
                    }))?
                    .build()),
            }
        })
        // POST /echo - Echo back the request body
        .post("/echo", |ctx| async move {
            let response_data = json!({
                "method": ctx.method.to_string(),
                "uri": ctx.uri.to_string(),
                "headers": ctx.headers.len(),
                "body_size": ctx.body.len(),
                "body_text": ctx.text().unwrap_or_else(|_| "<binary data>".to_string()),
                "timestamp": chrono::Utc::now().to_rfc3339()
            });
            HttpResponse::json(&response_data)
        })
        // PUT /config - Update configuration
        .put("/config", |ctx| async move {
            match ctx.json::<serde_json::Value>() {
                Ok(config_data) => {
                    info!("Received config update: {}", config_data);
                    HttpResponse::json(&json!({
                        "status": "updated",
                        "config": config_data
                    }))
                }
                Err(e) => Ok(HttpResponse::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .json(&json!({
                        "error": format!("Invalid config: {}", e)
                    }))?
                    .build()),
            }
        })
        // DELETE /cache - Clear cache
        .delete("/cache", |_ctx| async move {
            Ok(HttpResponse::builder()
                .status(http::StatusCode::NO_CONTENT)
                .build())
        });

    // Create and configure server
    #[cfg(unix)]
    let mut server = IpcHttpServer::with_config(&ipc_path, config)?
        .with_listener_mode(0o666) // Allow read/write for everyone
        .router(router);

    #[cfg(windows)]
    let mut server = IpcHttpServer::with_config(&ipc_path, config)?
        .with_listener_security_descriptor("D:(A;;GA;;;WD)") // Allow Everyone access
        .router(router);

    println!("🌟 Server configured with endpoints:");
    println!("  GET    /version     - API version information");
    println!("  GET    /health      - Health check");
    println!("  GET    /users       - List all users");
    println!("  GET    /user/1      - Get user by ID");
    println!("  POST   /users       - Create new user");
    println!("  POST   /echo        - Echo request data");
    println!("  PUT    /config      - Update configuration");
    println!("  DELETE /cache       - Clear cache");
    println!();

    // Start the server
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.serve().await {
            eprintln!("Server error: {}", e);
        }
    });

    println!("✅ Server started successfully!");
    println!("📊 Use the following commands to test:");
    println!();

    #[cfg(unix)]
    {
        println!("# Test basic endpoints:");
        println!(
            "CUSTOM_SOCK={} cargo run --features=client --example request",
            ipc_path
        );
        println!();
        println!("# Using curl-like tools with HTTP over IPC:");
        println!("# (These would work if you had an HTTP-to-IPC bridge)");
    }

    #[cfg(windows)]
    {
        println!("# Test basic endpoints:");
        println!("set CUSTOM_PIPE={}", ipc_path);
        println!("cargo run --features=client --example request");
        println!();
    }

    println!("# Example client requests you can make:");
    println!("GET /version     - Get server version");
    println!("GET /health      - Check server health");
    println!("GET /users       - List users");
    println!("POST /echo       - Echo request data");
    println!();

    // Wait for shutdown signal
    println!("🎯 Server is running. Press Ctrl+C to shutdown...");

    match signal::ctrl_c().await {
        Ok(()) => {
            println!("🛑 Shutdown signal received");
        }
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
        }
    }

    // Attempt graceful shutdown
    println!("🔄 Shutting down server...");
    server_handle.abort();

    // Give some time for cleanup
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("✅ Server stopped");

    Ok(())
}

// Helper function to demonstrate server statistics
#[allow(dead_code)]
async fn print_server_stats() {
    // This would be called periodically to show server stats
    // For now, it's just a placeholder
    info!("Server statistics would be displayed here");
}
