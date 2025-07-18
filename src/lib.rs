mod http_client;
mod stream_client;

pub mod errors;

pub mod ipc_http_client;
pub mod ipc_stream_client;

pub use ipc_http_client::IpcHttpClient;
pub use ipc_stream_client::IpcStreamClient;

pub use errors::{AnyError, AnyResult};

// Re-export types
pub use http_client::Response;
pub use stream_client::StreamingResponse;
