#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use self::unix::UnixIpcHttpClient as IpcHttpClient;
#[cfg(windows)]
pub use self::windows::WindowsIpcHttpClient as IpcHttpClient;
