use kode_bridge::{
    ipc_http_server::{HttpResponse, IpcHttpServer, Router},
    Result,
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    // Create HTTP server with routing
    let router = Router::new()
        .get("/version", |_| async move {
            HttpResponse::json(&json!({"version": "1.0.0"}))
        })
        .post("/api/data", |ctx| async move {
            let data: serde_json::Value = ctx.json()?;
            HttpResponse::json(&json!({"received": data}))
        });

    let mut server = IpcHttpServer::new("/tmp/server.sock")?.router(router);

    println!("🚀 Server listening on /tmp/server.sock");
    server.serve().await
}
