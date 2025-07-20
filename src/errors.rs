use std::fmt;
use thiserror::Error;

/// Comprehensive error types for kode-bridge
#[derive(Error, Debug)]
pub enum KodeBridgeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP parsing error: {0}")]
    HttpParse(#[from] httparse::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] http::Error),

    #[error("Invalid status code: {0}")]
    InvalidStatusCode(#[from] http::status::InvalidStatusCode),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("String conversion error: {0}")]
    FromUtf8(#[from] std::string::FromUtf8Error),

    #[error("Connection error: {message}")]
    Connection { message: String },

    #[error("Timeout error: operation timed out after {duration_ms}ms")]
    Timeout { duration_ms: u64 },

    #[error("Protocol error: {message}")]
    Protocol { message: String },

    #[error("Configuration error: {message}")]
    Configuration { message: String },

    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("Server error: HTTP {status}")]
    ServerError { status: u16 },

    #[error("Client error: HTTP {status}")]
    ClientError { status: u16 },

    #[error("Stream closed unexpectedly")]
    StreamClosed,

    #[error("JSON serialization error: {message}")]
    JsonSerialize { message: String },

    #[error("Pool exhausted: no available connections")]
    PoolExhausted,

    #[error("Custom error: {message}")]
    Custom { message: String },
}

impl KodeBridgeError {
    pub fn connection<S: Into<String>>(message: S) -> Self {
        Self::Connection {
            message: message.into(),
        }
    }

    pub fn timeout(duration_ms: u64) -> Self {
        Self::Timeout { duration_ms }
    }
    
    pub fn timeout_msg<S: Into<String>>(_message: S) -> Self {
        // Convert to a reasonable duration for now  
        Self::Timeout { duration_ms: 30000 }
    }

    pub fn protocol<S: Into<String>>(message: S) -> Self {
        Self::Protocol {
            message: message.into(),
        }
    }

    pub fn configuration<S: Into<String>>(message: S) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }

    pub fn invalid_request<S: Into<String>>(message: S) -> Self {
        Self::InvalidRequest {
            message: message.into(),
        }
    }

    pub fn custom<S: Into<String>>(message: S) -> Self {
        Self::Custom {
            message: message.into(),
        }
    }
    
    pub fn json_serialize<S: Into<String>>(message: S) -> Self {
        Self::JsonSerialize {
            message: message.into(),
        }
    }
    
    pub fn json_parse<S: Into<String>>(message: S) -> Self {
        Self::JsonSerialize {
            message: message.into(),
        }
    }
    
    pub fn validation<S: Into<String>>(message: S) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }

    /// Check if error is retriable
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::Io(_) | Self::Connection { .. } | Self::Timeout { .. } | Self::StreamClosed
        )
    }

    /// Check if error is a client error
    pub fn is_client_error(&self) -> bool {
        matches!(self, Self::ClientError { .. } | Self::InvalidRequest { .. })
    }

    /// Check if error is a server error
    pub fn is_server_error(&self) -> bool {
        matches!(self, Self::ServerError { .. })
    }
}

/// Result type for kode-bridge operations
pub type Result<T> = std::result::Result<T, KodeBridgeError>;

/// Legacy compatibility
pub type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type AnyResult<T> = std::result::Result<T, AnyError>;

/// Convert AnyError to KodeBridgeError
impl From<AnyError> for KodeBridgeError {
    fn from(err: AnyError) -> Self {
        KodeBridgeError::custom(err.to_string())
    }
}


/// Simple string error for backward compatibility
#[derive(Debug)]
pub struct ErrorString(String);

impl fmt::Display for ErrorString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ErrorString {}

impl From<String> for ErrorString {
    fn from(s: String) -> Self {
        ErrorString(s)
    }
}

impl From<&str> for ErrorString {
    fn from(s: &str) -> Self {
        ErrorString(s.to_string())
    }
}