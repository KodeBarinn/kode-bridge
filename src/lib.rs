mod config;
mod http_client;
mod pool;
mod response;
mod stream_client;

pub mod errors;

// Client modules (feature gated)
#[cfg(feature = "client")]
pub mod ipc_http_client;
#[cfg(feature = "client")]
pub mod ipc_stream_client;

// Server modules (feature gated)
#[cfg(feature = "server")]
pub mod ipc_http_server;
#[cfg(feature = "server")]
pub mod ipc_stream_server;

// Client re-exports
#[cfg(feature = "client")]
pub use ipc_http_client::IpcHttpClient;
#[cfg(feature = "client")]
pub use ipc_stream_client::IpcStreamClient;

// Server re-exports
#[cfg(feature = "server")]
pub use ipc_http_server::{
    ClientInfo, HttpResponse, IpcHttpServer, RequestContext, ResponseBuilder, Router, ServerConfig,
    ServerStats,
};
#[cfg(feature = "server")]
pub use ipc_stream_server::{
    IpcStreamServer, IteratorSource, JsonDataSource, StreamClient, StreamMessage,
    StreamServerConfig, StreamServerStats, StreamSource,
};

// Error types
pub use errors::{AnyError, AnyResult, KodeBridgeError, Result};

// Response types
pub use http_client::Response;
pub use response::LegacyResponse;
pub use stream_client::StreamingResponse;

// Pool types
pub use pool::{ConnectionPool, PoolConfig, PoolStats, PooledConnection};

// Configuration types
pub use config::{
    ClientGlobalConfig, ConfigBuilder, FeatureFlags, GlobalConfig, LoggingConfig,
    StreamingGlobalConfig,
};

// Client configuration re-exports
#[cfg(feature = "client")]
pub use ipc_http_client::ClientConfig;
#[cfg(feature = "client")]
pub use ipc_stream_client::StreamClientConfig;
