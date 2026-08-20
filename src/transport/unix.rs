#[cfg(feature = "server")]
use super::ListenerOptions;
#[cfg(feature = "server")]
use std::fs::Metadata;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
#[cfg(all(feature = "server", not(target_os = "macos")))]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(feature = "server")]
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
#[cfg(feature = "server")]
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
#[cfg(feature = "server")]
use std::path::PathBuf;
#[cfg(feature = "server")]
use std::time::Instant;
use tokio::net::UnixStream;
#[cfg(feature = "server")]
use tokio::net::{UnixListener, UnixSocket};

pub(super) type ClientStream = UnixStream;
#[cfg(feature = "server")]
pub(crate) type ServerStream = UnixStream;

pub(super) fn validate_endpoint(path: &Path) -> io::Result<()> {
    if path.as_os_str().as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "filesystem paths cannot contain interior nuls",
        ));
    }
    Ok(())
}

pub(super) async fn connect(path: &Path) -> io::Result<ClientStream> {
    UnixStream::connect(path).await
}

#[cfg(feature = "server")]
pub(crate) struct Listener {
    inner: UnixListener,
    _path_guard: Option<SocketPathGuard>,
}

#[cfg(feature = "server")]
impl Listener {
    pub(crate) fn bind(path: &Path, options: &ListenerOptions) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        if options.mode.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unix listener mode is unsupported on macOS",
            ));
        }

        let socket = bind_socket(path, options)?;
        let metadata = socket_metadata(path)?;
        let path_guard = options
            .reclaim_name
            .then(|| SocketPathGuard::new(path.to_path_buf(), &metadata));

        #[cfg(not(target_os = "macos"))]
        if let Some(mode) = options.mode {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        }

        let current = socket_metadata(path)?;
        if !same_file(&metadata, &current) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unix socket path changed before listen",
            ));
        }

        let inner = socket.listen(1024)?;
        Ok(Self {
            inner,
            _path_guard: path_guard,
        })
    }

    // The Windows backend mutates its pending pipe instance during accept, so the shared
    // transport contract intentionally keeps a mutable receiver on every platform.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub(crate) async fn accept(&mut self) -> io::Result<ServerStream> {
        self.inner.accept().await.map(|(stream, _address)| stream)
    }
}

#[cfg(feature = "server")]
fn socket_metadata(path: &Path) -> io::Result<Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "Unix socket path no longer refers to a socket",
        ));
    }
    Ok(metadata)
}

#[cfg(feature = "server")]
fn bind_socket(path: &Path, options: &ListenerOptions) -> io::Result<UnixSocket> {
    let started = Instant::now();
    let mut attempted_reclaim = false;
    loop {
        let socket = UnixSocket::new_stream()?;
        match socket.bind(path) {
            Ok(()) => return Ok(socket),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse && options.try_overwrite => {
                if attempted_reclaim
                    && options
                        .max_spin_time
                        .is_some_and(|limit| started.elapsed() >= limit)
                {
                    return Err(error);
                }
                reclaim_stale_socket(path)?;
                attempted_reclaim = true;
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(feature = "server")]
fn reclaim_stale_socket(path: &Path) -> io::Result<()> {
    let before = std::fs::symlink_metadata(path)?;
    if !before.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "refusing to replace a non-socket endpoint",
        ));
    }

    match StdUnixStream::connect(path) {
        Ok(_live) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "refusing to replace a live Unix listener",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Err(error) => return Err(error),
    }

    let after = std::fs::symlink_metadata(path)?;
    if !same_file(&before, &after) || !after.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "endpoint changed while checking stale socket",
        ));
    }
    std::fs::remove_file(path)
}

#[cfg(feature = "server")]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(feature = "server")]
struct SocketPathGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(feature = "server")]
impl SocketPathGuard {
    fn new(path: PathBuf, metadata: &Metadata) -> Self {
        Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(feature = "server")]
impl Drop for SocketPathGuard {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket() && metadata.dev() == self.device && metadata.ino() == self.inode {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn path(label: &str) -> PathBuf {
        let sequence = PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "/tmp/kb-transport-{}-{sequence}-{label}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn overwrite_reclaims_only_stale_sockets() -> io::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        let _runtime_guard = runtime.enter();
        let stale_path = path("stale");
        let stale = std::os::unix::net::UnixListener::bind(&stale_path)?;
        drop(stale);

        let listener = Listener::bind(
            &stale_path,
            &ListenerOptions::new()
                .try_overwrite(true)
                .max_spin_time(std::time::Duration::ZERO),
        )?;
        drop(listener);
        assert!(!stale_path.exists());

        let file_path = path("file");
        std::fs::write(&file_path, b"sentinel")?;
        let result = Listener::bind(&file_path, &ListenerOptions::new().try_overwrite(true));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&file_path)?, b"sentinel");
        std::fs::remove_file(file_path)?;
        Ok(())
    }

    #[test]
    fn overwrite_refuses_live_listener() -> io::Result<()> {
        let live_path = path("live");
        let live = std::os::unix::net::UnixListener::bind(&live_path)?;
        let result = Listener::bind(&live_path, &ListenerOptions::new().try_overwrite(true));
        assert!(result.is_err());
        assert!(live_path.exists());
        drop(live);
        std::fs::remove_file(live_path)?;
        Ok(())
    }

    #[test]
    fn guard_does_not_remove_replaced_socket() -> io::Result<()> {
        let guarded_path = path("guard");
        let original = std::os::unix::net::UnixListener::bind(&guarded_path)?;
        let metadata = std::fs::symlink_metadata(&guarded_path)?;
        let guard = SocketPathGuard::new(guarded_path.clone(), &metadata);
        drop(original);
        std::fs::remove_file(&guarded_path)?;
        let replacement = std::os::unix::net::UnixListener::bind(&guarded_path)?;

        drop(guard);
        assert!(guarded_path.exists());
        drop(replacement);
        std::fs::remove_file(guarded_path)?;
        Ok(())
    }
}
