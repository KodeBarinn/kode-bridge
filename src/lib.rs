mod http_client;
mod stream_client;
mod pool;
mod response;
mod config;

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
pub use ipc_http_server::{IpcHttpServer, Router, RequestContext, HttpResponse, ResponseBuilder, ServerConfig, ServerStats, ClientInfo};
#[cfg(feature = "server")]
pub use ipc_stream_server::{IpcStreamServer, StreamMessage, StreamClient, StreamServerConfig, StreamServerStats, StreamSource, JsonDataSource, IteratorSource};

// Error types
pub use errors::{KodeBridgeError, Result, AnyError, AnyResult};

// Response types
pub use http_client::Response;
pub use response::LegacyResponse;
pub use stream_client::StreamingResponse;

// Pool types
pub use pool::{ConnectionPool, PoolConfig, PoolStats, PooledConnection};

// Configuration types
pub use config::{GlobalConfig, ConfigBuilder, ClientGlobalConfig, StreamingGlobalConfig, LoggingConfig, FeatureFlags};

// Client configuration re-exports
#[cfg(feature = "client")]
pub use ipc_http_client::ClientConfig;
#[cfg(feature = "client")]
pub use ipc_stream_client::StreamClientConfig;
