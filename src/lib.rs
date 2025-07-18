mod http_client;
mod stream_client;

pub mod errors;
pub mod types;

pub mod ipc_http_client;
pub mod ipc_stream_client;

pub use ipc_http_client::IpcHttpClient;
pub use ipc_stream_client::{ConnectionData, IpcStreamClient, TrafficData};

pub use errors::{AnyError, AnyResult};

// Re-export streaming types
pub use stream_client::StreamingResponse;
