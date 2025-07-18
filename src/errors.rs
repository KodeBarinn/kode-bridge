use std::error::Error;
use std::fmt::{Display, Formatter};

/// Universal error type that can wrap any error
pub type AnyError = Box<dyn Error + Send + Sync + 'static>;

/// Universal result type
pub type AnyResult<T> = Result<T, AnyError>;

/// Simple string error
#[derive(Debug)]
pub struct ErrorString(String);

impl Display for ErrorString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ErrorString {}

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
