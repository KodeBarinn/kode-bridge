use std::error::Error;
use std::fmt::{Debug, Display};

/// A trait for any error that can be converted to a boxed error
pub trait AnyError: Error + Send + Sync + Debug + Display {
    /// Convert the error to a boxed standard error
    fn into_std_box(self) -> Box<dyn Error + Send + Sync>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }
}

/// Blanket implementation for all types that implement the required traits
impl<T> AnyError for T where T: Error + Send + Sync + Debug + Display {}

/// A result type that uses AnyError as the error type
pub type AnyResult<T> = Result<T, Box<dyn AnyError>>;

/// Helper trait to convert any error to AnyError
pub trait IntoAnyError<T> {
    fn into_any_error(self) -> AnyResult<T>;
}

impl<T, E> IntoAnyError<T> for Result<T, E>
where
    E: AnyError + 'static,
{
    fn into_any_error(self) -> AnyResult<T> {
        self.map_err(|e| Box::new(e) as Box<dyn AnyError>)
    }
}

/// Create an AnyError from any error type
pub fn any_error<E>(error: E) -> Box<dyn AnyError>
where
    E: AnyError + 'static,
{
    Box::new(error)
}

/// Convert any error to AnyError (convenience function)
pub fn to_any_error<E>(error: E) -> Box<dyn AnyError>
where
    E: AnyError + 'static,
{
    Box::new(error)
}

/// Generic From implementation for any error type that implements AnyError
/// (excludes Box<dyn Error + Send + Sync> to avoid conflicts)
impl<E> From<E> for Box<dyn AnyError>
where
    E: AnyError + 'static,
{
    fn from(error: E) -> Self {
        Box::new(error)
    }
}
