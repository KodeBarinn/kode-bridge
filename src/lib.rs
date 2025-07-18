mod http_client;
mod stream_client;

pub mod errors;
pub mod types;

pub mod ipc_client;
pub use ipc_client::IpcHttpClient;

pub use errors::{AnyError, AnyResult};

// Re-export streaming types
pub use stream_client::StreamingResponse;
