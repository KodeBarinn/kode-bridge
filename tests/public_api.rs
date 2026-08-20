#![cfg(feature = "server")]

use kode_bridge::{
    pool::{ConnectionPool, PoolConfig, PooledConnection},
    Endpoint, IpcHttpServer, IpcStream, IpcStreamServer, ListenerOptions,
};
use tokio::io::{AsyncRead, AsyncWrite};

const fn assert_async_io<T: AsyncRead + AsyncWrite + Unpin + Send>() {}

fn pooled_stream_signatures(mut connection: PooledConnection) {
    let _borrowed: Option<&mut IpcStream> = connection.stream();
    let _owned: Option<IpcStream> = connection.into_stream();
}

#[test]
fn migration_api_contract_compiles() -> kode_bridge::Result<()> {
    #[cfg(unix)]
    let path = "/tmp/kode-bridge-public-api.sock";
    #[cfg(windows)]
    let path = r"\\.\pipe\kode-bridge-public-api";

    let endpoint = Endpoint::new(path)?;
    assert_eq!(endpoint.as_path(), std::path::Path::new(path));

    let listener_options = ListenerOptions::new()
        .reclaim_name(true)
        .try_overwrite(false)
        .max_spin_time(std::time::Duration::from_millis(100));
    let _server = IpcHttpServer::new(endpoint.as_path())?.with_listener_options(listener_options.clone());
    let _stream_server = IpcStreamServer::new(endpoint.as_path())?.with_listener_options(listener_options);

    let _pool = ConnectionPool::new(endpoint.clone(), PoolConfig::default());
    let _default_pool = ConnectionPool::with_default_config(endpoint);
    let _pooled_api: fn(PooledConnection) = pooled_stream_signatures;
    assert_async_io::<IpcStream>();
    Ok(())
}
