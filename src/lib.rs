mod http_client;
mod stream_client;
mod pool;
mod response;
mod config;

pub mod errors;
pub mod ipc_http_client;
pub mod ipc_stream_client;

// Re-exports
pub use ipc_http_client::IpcHttpClient;
pub use ipc_stream_client::IpcStreamClient;

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
pub use ipc_http_client::ClientConfig;
pub use ipc_stream_client::StreamClientConfig;
