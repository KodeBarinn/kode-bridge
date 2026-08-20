use crate::errors::{KodeBridgeError, Result};
use std::io::IoSlice;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// A validated cross-platform IPC endpoint.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Endpoint(PathBuf);

impl Endpoint {
    /// Validate and own an IPC endpoint path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        platform::validate_endpoint(path.as_ref())
            .map_err(|error| KodeBridgeError::configuration(format!("Invalid IPC endpoint: {error}")))?;
        Ok(Self(path.as_ref().to_path_buf()))
    }

    /// Return the platform endpoint path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Endpoint {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

/// Cross-platform IPC listener configuration.
#[derive(Clone, Debug)]
pub struct ListenerOptions {
    reclaim_name: bool,
    try_overwrite: bool,
    #[cfg(unix)]
    max_spin_time: Option<Duration>,
    #[cfg(unix)]
    mode: Option<libc::mode_t>,
    #[cfg(windows)]
    security_descriptor: Option<std::sync::Arc<windows::SecurityDescriptor>>,
}

impl ListenerOptions {
    /// Return the default listener configuration.
    pub const fn new() -> Self {
        Self {
            reclaim_name: true,
            try_overwrite: false,
            #[cfg(unix)]
            max_spin_time: None,
            #[cfg(unix)]
            mode: None,
            #[cfg(windows)]
            security_descriptor: None,
        }
    }

    /// Configure whether the endpoint name is removed when the listener drops.
    #[must_use]
    pub const fn reclaim_name(mut self, reclaim_name: bool) -> Self {
        self.reclaim_name = reclaim_name;
        self
    }

    /// Configure whether a proven-stale Unix socket may be replaced while binding.
    #[must_use]
    pub const fn try_overwrite(mut self, try_overwrite: bool) -> Self {
        self.try_overwrite = try_overwrite;
        self
    }

    /// Bound retry time for Unix stale-socket replacement contention.
    #[must_use]
    #[cfg_attr(not(unix), allow(unused_mut))]
    pub const fn max_spin_time(mut self, max_spin_time: Duration) -> Self {
        #[cfg(unix)]
        {
            self.max_spin_time = Some(max_spin_time);
        }
        let _ = max_spin_time;
        self
    }

    /// Set Unix socket permissions before the listener begins accepting clients.
    #[cfg(unix)]
    #[must_use]
    pub const fn mode(mut self, mode: libc::mode_t) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Set a Windows named-pipe security descriptor from SDDL.
    ///
    /// # Panics
    /// Panics with `Invalid SDDL string` for an interior NUL, or with
    /// `Failed to parse SDDL` when Windows rejects the descriptor.
    #[cfg(windows)]
    #[must_use]
    #[allow(clippy::panic)]
    pub fn security_descriptor(mut self, sddl: &str) -> Self {
        if sddl.encode_utf16().any(|unit| unit == 0) {
            panic!("Invalid SDDL string");
        }
        let descriptor = match windows::SecurityDescriptor::from_sddl(sddl) {
            Ok(descriptor) => descriptor,
            Err(error) => panic!("Failed to parse SDDL: {error}"),
        };
        self.security_descriptor = Some(std::sync::Arc::new(descriptor));
        self
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::ListenerOptions;

    #[test]
    #[should_panic(expected = "Invalid SDDL string")]
    fn interior_nul_keeps_legacy_panic_classification() {
        let _options = ListenerOptions::new().security_descriptor("D:\0(A;;GA;;;WD)");
    }

    #[test]
    #[should_panic(expected = "Failed to parse SDDL")]
    fn malformed_sddl_keeps_legacy_panic_classification() {
        let _options = ListenerOptions::new().security_descriptor("not-valid-sddl");
    }
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Client-side stream returned by kode-bridge IPC connections.
#[derive(Debug)]
pub struct IpcStream(platform::ClientStream);

impl IpcStream {
    pub(crate) async fn connect(endpoint: &Endpoint) -> std::io::Result<Self> {
        platform::connect(endpoint.as_path()).await.map(Self)
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buffer)
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buffer)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

#[cfg(feature = "server")]
pub(crate) type ServerStream = platform::ServerStream;

#[cfg(feature = "server")]
pub(crate) struct Listener(platform::Listener);

#[cfg(feature = "server")]
impl Listener {
    pub(crate) fn bind(endpoint: &Endpoint, options: &ListenerOptions) -> std::io::Result<Self> {
        platform::Listener::bind(endpoint.as_path(), options).map(Self)
    }

    pub(crate) async fn accept(&mut self) -> std::io::Result<ServerStream> {
        self.0.accept().await
    }
}
