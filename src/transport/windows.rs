#[cfg(feature = "server")]
use super::ListenerOptions;
use super::WindowsServerVerification;
use std::ffi::{c_void, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{self, IoSlice};
use std::iter;
#[cfg(feature = "server")]
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsHandle, AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::{
    mpsc::{sync_channel, SyncSender, TrySendError},
    Arc, Mutex, OnceLock,
};
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
#[cfg(feature = "server")]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{LocalFree, ERROR_PIPE_BUSY};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
#[cfg(feature = "server")]
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, GetTokenInformation, TokenUser, WinLocalSystemSid, SECURITY_MAX_SID_SIZE,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

pub(super) type ClientStream = PipeStream<NamedPipeClient>;
#[cfg(feature = "server")]
pub(crate) type ServerStream = PipeStream<NamedPipeServer>;

const FLUSH_QUEUE_CAPACITY: usize = 64;

// Explicit flushes use per-stream blocking tasks so one slow reader cannot stall
// another connection. Drop linger uses the bounded queue below; saturation above
// the 0.4 high watermark moves that job to an independent worker, while server
// max_connections bounds normal queue exposure.
pub(crate) struct PipeStream<T: AsHandle> {
    inner: T,
    dirty: bool,
    flush: Option<tokio::task::JoinHandle<io::Result<()>>>,
}

impl<T: AsHandle> PipeStream<T> {
    const fn new(inner: T) -> Self {
        Self {
            inner,
            dirty: false,
            flush: None,
        }
    }

    fn poll_pending_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Some(flush) = &mut self.flush else {
            return Poll::Ready(Ok(()));
        };
        match std::future::Future::poll(Pin::new(flush), cx) {
            Poll::Ready(Ok(Ok(()))) => {
                self.flush = None;
                self.dirty = false;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Ok(Err(error))) => {
                self.flush = None;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Err(error)) => {
                self.flush = None;
                Poll::Ready(Err(io::Error::other(format!("named pipe flush task failed: {error}"))))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn begin_flush(&mut self) -> io::Result<()> {
        let owned_handle = self.inner.as_handle().try_clone_to_owned()?;
        self.flush = Some(tokio::task::spawn_blocking(move || File::from(owned_handle).sync_all()));
        Ok(())
    }
}

impl<T> fmt::Debug for PipeStream<T>
where
    T: AsHandle + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PipeStream")
            .field("inner", &self.inner)
            .field("dirty", &self.dirty)
            .field("flush_pending", &self.flush.is_some())
            .finish()
    }
}

impl<T> AsyncRead for PipeStream<T>
where
    T: AsHandle + AsyncRead + Unpin,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}

impl<T> AsyncWrite for PipeStream<T>
where
    T: AsHandle + AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.poll_pending_flush(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        }

        match Pin::new(&mut this.inner).poll_write(cx, buffer) {
            Poll::Ready(Ok(written)) => {
                this.dirty |= written != 0;
                Poll::Ready(Ok(written))
            }
            result => result,
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.poll_pending_flush(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        }

        match Pin::new(&mut this.inner).poll_write_vectored(cx, buffers) {
            Poll::Ready(Ok(written)) => {
                this.dirty |= written != 0;
                Poll::Ready(Ok(written))
            }
            result => result,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.poll_pending_flush(cx) {
            Poll::Ready(Ok(())) => {}
            result => return result,
        }
        if this.dirty {
            if let Err(error) = this.begin_flush() {
                return Poll::Ready(Err(error));
            }
        }
        this.poll_pending_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut self.get_mut().inner).poll_shutdown(cx),
            result => result,
        }
    }
}

impl<T: AsHandle> Drop for PipeStream<T> {
    fn drop(&mut self) {
        if self.dirty {
            // A deliberate duplicate covers pending, cancelled, and failed-but-unobserved
            // flushes. Each job owns an independent safe handle and may run concurrently.
            let _ = enqueue_linger(&self.inner);
        }
    }
}

struct FlushJob {
    file: File,
}

impl FlushJob {
    fn run(self) {
        let _ = self.file.sync_all();
    }
}

fn enqueue_linger(stream: &impl AsHandle) -> io::Result<()> {
    let owned_handle = stream.as_handle().try_clone_to_owned()?;
    let job = FlushJob {
        file: File::from(owned_handle),
    };

    match flush_sender().try_send(job) {
        Ok(()) => {}
        Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => spawn_fallback(job),
    }
    Ok(())
}

fn flush_sender() -> &'static SyncSender<FlushJob> {
    static FLUSH_SENDER: OnceLock<SyncSender<FlushJob>> = OnceLock::new();
    FLUSH_SENDER.get_or_init(|| {
        let (sender, receiver) = sync_channel::<FlushJob>(FLUSH_QUEUE_CAPACITY);
        let _ = thread::Builder::new()
            .name("kode-bridge-pipe-flush".to_string())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job.run();
                }
            });
        sender
    })
}

fn spawn_fallback(job: FlushJob) {
    let job = Arc::new(Mutex::new(Some(job)));
    let worker_job = Arc::clone(&job);
    if thread::Builder::new()
        .name("kode-bridge-pipe-flush-fallback".to_string())
        .spawn(move || run_flush_job(&worker_job))
        .is_err()
    {
        run_flush_job(&job);
    }
}

fn run_flush_job(job: &Mutex<Option<FlushJob>>) {
    let job = {
        let mut job = job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let owned_job = job.take();
        drop(job);
        owned_job
    };
    if let Some(job) = job {
        job.run();
    }
}

pub(super) fn validate_endpoint(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let Some(host_end) = bytes
        .get(2..)
        .and_then(|rest| rest.iter().position(|byte| *byte == b'\\'))
    else {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "not a named pipe path"));
    };
    let pipe_prefix = 2 + host_end;
    if !bytes.starts_with(br"\\")
        || bytes.get(pipe_prefix..pipe_prefix + 6) != Some(br"\pipe\")
        || bytes.len() <= pipe_prefix + 6
        || path.as_os_str().encode_wide().any(|unit| unit == 0)
    {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "not a named pipe path"));
    }
    Ok(())
}

pub(super) async fn connect(path: &Path) -> io::Result<ClientStream> {
    connect_with_server_verification(path, WindowsServerVerification::default()).await
}

pub(super) async fn connect_with_server_verification(
    path: &Path,
    verification: WindowsServerVerification,
) -> io::Result<ClientStream> {
    loop {
        match ClientOptions::new().open(path) {
            Ok(client) => {
                verify_server(&client, verification)?;
                return Ok(PipeStream::new(client));
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn verify_server(client: &NamedPipeClient, verification: WindowsServerVerification) -> io::Result<()> {
    if !verification.require_system && verification.verifier.is_none() {
        return Ok(());
    }

    let process_id = pipe_server_process_id(client)?;
    if verification.require_system {
        verify_process_is_local_system(process_id)?;
    }
    if let Some(verifier) = verification.verifier {
        verifier(process_id)?;
    }
    Ok(())
}

fn pipe_server_process_id(client: &NamedPipeClient) -> io::Result<u32> {
    let mut process_id = 0_u32;
    if unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut process_id) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if process_id == 0 {
        return Err(io::Error::other(
            "Windows named-pipe server returned an invalid process ID",
        ));
    }
    Ok(process_id)
}

fn verify_process_is_local_system(process_id: u32) -> io::Result<()> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process) };

    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    let mut required = 0_u32;
    unsafe {
        GetTokenInformation(token.as_raw_handle(), TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let words = (required as usize).div_ceil(std::mem::size_of::<usize>());
    let mut user = vec![0_usize; words];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*user.as_ptr().cast::<TOKEN_USER>() };

    let sid_words = (SECURITY_MAX_SID_SIZE as usize).div_ceil(std::mem::size_of::<usize>());
    let mut system_sid = vec![0_usize; sid_words];
    let mut system_sid_size = SECURITY_MAX_SID_SIZE;
    if unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            std::ptr::null_mut(),
            system_sid.as_mut_ptr().cast(),
            &mut system_sid_size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if unsafe { EqualSid(user.User.Sid, system_sid.as_mut_ptr().cast()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Windows named-pipe server is not LocalSystem",
        ));
    }
    Ok(())
}

#[cfg(feature = "server")]
pub(crate) struct Listener {
    path: std::path::PathBuf,
    options: ListenerOptions,
    pending: Option<NamedPipeServer>,
}

#[cfg(feature = "server")]
impl Listener {
    pub(crate) fn bind(path: &Path, options: &ListenerOptions) -> io::Result<Self> {
        let pending = create_instance(path, options, true)?;
        Ok(Self {
            path: path.to_path_buf(),
            options: options.clone(),
            pending: Some(pending),
        })
    }

    pub(crate) async fn accept(&mut self) -> io::Result<ServerStream> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| io::Error::other("named pipe listener is unavailable"))?;
        pending.connect().await?;

        let next = create_instance(&self.path, &self.options, false)?;
        let connected = self
            .pending
            .replace(next)
            .ok_or_else(|| io::Error::other("named pipe listener is unavailable"))?;
        Ok(PipeStream::new(connected))
    }
}

#[cfg(feature = "server")]
fn create_instance(path: &Path, options: &ListenerOptions, first: bool) -> io::Result<NamedPipeServer> {
    let mut server_options = ServerOptions::new();
    server_options
        .first_pipe_instance(first)
        .reject_remote_clients(true)
        .in_buffer_size(512)
        .out_buffer_size(512);

    match &options.security_descriptor {
        Some(descriptor) => {
            let mut attributes = descriptor.security_attributes();
            // SAFETY: `attributes` is valid for this synchronous call, and its descriptor is
            // owned by `options`, which outlives every pipe instance created by this listener.
            unsafe {
                server_options.create_with_security_attributes_raw(
                    path,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
                )
            }
        }
        None => server_options.create(path),
    }
}

#[derive(Debug)]
pub(super) struct SecurityDescriptor {
    pointer: NonNull<c_void>,
}

impl SecurityDescriptor {
    pub(super) fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut encoded = OsStr::new(sddl).encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SDDL cannot contain interior nuls",
            ));
        }
        encoded.extend(iter::once(0));
        let mut pointer = std::ptr::null_mut();
        // SAFETY: `encoded` is nul-terminated and remains alive for the call. Windows initializes
        // `pointer` with LocalAlloc memory on success; `Drop` releases it with `LocalFree`.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                encoded.as_ptr(),
                SDDL_REVISION_1,
                &mut pointer,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let pointer = NonNull::new(pointer)
            .ok_or_else(|| io::Error::other("SDDL conversion returned a null security descriptor"))?;
        Ok(Self { pointer })
    }

    #[cfg(feature = "server")]
    const fn security_attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.pointer.as_ptr(),
            bInheritHandle: 0,
        }
    }
}

// SAFETY: the descriptor allocation is immutable after construction and remains valid until Drop.
unsafe impl Send for SecurityDescriptor {}
// SAFETY: shared access only reads the immutable descriptor pointer for CreateNamedPipeW.
unsafe impl Sync for SecurityDescriptor {}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the pointer was allocated by the SDDL conversion API and is freed exactly once.
        let _ = unsafe { LocalFree(self.pointer.as_ptr()) };
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::oneshot;

    static PIPE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn pipe_path(label: &str) -> String {
        let sequence = PIPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!(r"\\.\pipe\kode-bridge-{}-{sequence}-{label}", std::process::id())
    }

    async fn connected_pair(label: &str) -> io::Result<(ServerStream, ClientStream)> {
        let path = pipe_path(label);
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .in_buffer_size(512)
            .out_buffer_size(512);
        let raw_server = options.create(&path)?;
        let server_task = tokio::spawn(async move {
            raw_server.connect().await?;
            Ok::<_, io::Error>(PipeStream::new(raw_server))
        });
        let client = connect(Path::new(&path)).await?;
        let server = server_task.await.map_err(io::Error::other)??;
        Ok((server, client))
    }

    fn inject_pending_flush<T: AsHandle>(stream: &mut PipeStream<T>) -> oneshot::Sender<()> {
        let (release_tx, release_rx) = oneshot::channel();
        stream.flush = Some(tokio::spawn(async move {
            let _ = release_rx.await;
            Ok(())
        }));
        release_tx
    }

    async fn flush_and_read_payload(
        server: &mut ServerStream,
        client: &mut ClientStream,
        payload: &[u8],
        read_delay: Duration,
    ) -> io::Result<()> {
        let mut received = vec![0; payload.len()];
        let complete = async {
            tokio::try_join!(server.flush(), async {
                tokio::time::sleep(read_delay).await;
                client.read_exact(&mut received).await
            })?;
            Ok::<(), io::Error>(())
        };
        match tokio::time::timeout(Duration::from_secs(5), complete).await {
            Ok(result) => result?,
            Err(_elapsed) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "pipe flush and client read did not complete within the deadline",
                ));
            }
        }
        assert_eq!(received, payload);
        Ok(())
    }

    async fn read_payload(client: &mut ClientStream, payload: &[u8]) -> io::Result<()> {
        let mut received = vec![0; payload.len()];
        match tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut received)).await {
            Ok(result) => {
                result?;
            }
            Err(_elapsed) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "client did not receive the complete named-pipe payload",
                ));
            }
        }
        assert_eq!(received, payload);
        Ok(())
    }

    #[tokio::test]
    async fn explicit_flush_and_client_read_complete_with_payload() -> io::Result<()> {
        let (mut server, mut client) = connected_pair("flush").await?;
        let payload = b"final response";
        server.write_all(payload).await?;

        flush_and_read_payload(&mut server, &mut client, payload, Duration::from_millis(50)).await
    }

    #[tokio::test]
    async fn explicit_flushes_do_not_block_other_connections() -> io::Result<()> {
        let (mut server_one, mut client_one) = connected_pair("independent-flush-one").await?;
        let (mut server_two, mut client_two) = connected_pair("independent-flush-two").await?;
        let payload_one = b"first slow response";
        let payload_two = b"second independent response";
        server_one.write_all(payload_one).await?;
        server_two.write_all(payload_two).await?;

        let release_one = inject_pending_flush(&mut server_one);
        let mut flush_one = Box::pin(server_one.flush());
        let flush_one_pending =
            std::future::poll_fn(|cx| Poll::Ready(std::future::Future::poll(flush_one.as_mut(), cx).is_pending()))
                .await;
        assert!(flush_one_pending, "injected first flush did not remain pending");

        flush_and_read_payload(&mut server_two, &mut client_two, payload_two, Duration::ZERO).await?;

        let first_still_pending =
            std::future::poll_fn(|cx| Poll::Ready(std::future::Future::poll(flush_one.as_mut(), cx).is_pending()))
                .await;
        assert!(first_still_pending, "injected first flush completed before release");
        let _ = release_one.send(());
        match tokio::time::timeout(Duration::from_secs(5), &mut flush_one).await {
            Ok(result) => result?,
            Err(_elapsed) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "injected first flush did not complete after release",
                ));
            }
        }
        read_payload(&mut client_one, payload_one).await
    }

    #[tokio::test]
    async fn dirty_drop_lingers_until_the_client_reads() -> io::Result<()> {
        let (mut server, mut client) = connected_pair("dirty-drop").await?;
        let payload = b"complete response after direct drop";
        server.write_all(payload).await?;
        drop(server);

        read_payload(&mut client, payload).await
    }

    #[tokio::test]
    async fn cancelled_flush_then_drop_still_lingers() -> io::Result<()> {
        let (mut server, mut client) = connected_pair("cancelled-flush").await?;
        let payload = b"complete response after cancelled flush";
        server.write_all(payload).await?;

        let release = inject_pending_flush(&mut server);
        let mut flush = Box::pin(server.flush());
        let pending =
            std::future::poll_fn(|cx| Poll::Ready(std::future::Future::poll(flush.as_mut(), cx).is_pending())).await;
        assert!(pending, "injected flush did not remain pending");
        drop(flush);
        drop(server);
        drop(release);

        read_payload(&mut client, payload).await
    }

    #[tokio::test]
    async fn flush_error_retains_dirty_state_for_retry() -> io::Result<()> {
        let (mut server, _client) = connected_pair("flush-error").await?;
        server.dirty = true;
        server.flush = Some(tokio::task::spawn_blocking(|| {
            Err(io::Error::other("injected flush failure"))
        }));

        let result = std::future::poll_fn(|cx| server.poll_pending_flush(cx)).await;
        assert!(result.is_err());
        assert!(server.dirty);
        assert!(server.flush.is_none());
        server.dirty = false;
        Ok(())
    }

    #[tokio::test]
    async fn thirty_two_concurrent_flushes_and_reads_complete_without_hanging() -> io::Result<()> {
        const CLIENTS: usize = 32;
        let payload = b"concurrent response";
        let mut clients = Vec::with_capacity(CLIENTS);
        let mut started_receivers = Vec::with_capacity(CLIENTS);
        let mut flush_tasks = Vec::with_capacity(CLIENTS);

        for index in 0..CLIENTS {
            let (mut server, client) = connected_pair(&format!("stress-{index}")).await?;
            let (started_tx, started_rx) = oneshot::channel();
            let flush_task = tokio::spawn(async move {
                server.write_all(payload).await?;
                let mut flush = Box::pin(server.flush());
                let mut started_tx = Some(started_tx);
                std::future::poll_fn(|cx| {
                    let result = std::future::Future::poll(flush.as_mut(), cx);
                    if let Some(started_tx) = started_tx.take() {
                        let _ = started_tx.send(());
                    }
                    result
                })
                .await
            });
            clients.push(client);
            started_receivers.push(started_rx);
            flush_tasks.push(flush_task);
        }

        let complete = async {
            for started in futures::future::join_all(started_receivers).await {
                started.map_err(io::Error::other)?;
            }

            let reads = clients
                .into_iter()
                .map(|mut client| async move { read_payload(&mut client, payload).await });
            for result in futures::future::join_all(reads).await {
                result?;
            }

            for result in futures::future::join_all(flush_tasks).await {
                result.map_err(io::Error::other)??;
            }
            Ok::<(), io::Error>(())
        };
        match tokio::time::timeout(Duration::from_secs(10), complete).await {
            Ok(result) => result,
            Err(_elapsed) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "concurrent named-pipe flushes and reads did not complete within the deadline",
            )),
        }
    }
}
