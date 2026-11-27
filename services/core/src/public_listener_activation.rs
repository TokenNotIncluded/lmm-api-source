use std::{
    io,
    net::{SocketAddr, TcpListener},
};

/// Called only by the server entry point, before starting any runtime threads.
/// The descriptor library consumes the socket-activation environment variables.
#[cfg(target_os = "linux")]
pub fn take() -> io::Result<Option<TcpListener>> {
    use std::{env, ffi::OsStr};

    let pid = env::var_os("LISTEN_PID");
    let count = env::var_os("LISTEN_FDS");
    let names = env::var_os("LISTEN_FDNAMES");
    let offset = env::var_os("LISTEN_FDS_FIRST_FD");
    if pid.is_none() && count.is_none() && names.is_none() && offset.is_none() {
        return Ok(None);
    }
    if pid
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse().ok())
        != Some(std::process::id())
        || count.as_deref() != Some(OsStr::new("1"))
        || names.as_deref() != Some(OsStr::new("lmm-core-http"))
        || offset.is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one systemd public socket named lmm-core-http for this process",
        ));
    }
    let listener = listenfd::ListenFd::from_env()
        .take_tcp_listener(0)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "public socket is missing"))?;
    if !socket2::SockRef::from(&listener).is_listener()? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "systemd public socket is not listening",
        ));
    }
    listener.set_nonblocking(true)?;
    Ok(Some(listener))
}

#[cfg(not(target_os = "linux"))]
pub fn take() -> io::Result<Option<TcpListener>> {
    Ok(None)
}

pub fn into_runtime(
    listener: TcpListener,
    address: SocketAddr,
) -> io::Result<tokio::net::TcpListener> {
    if listener.local_addr()? != address {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "systemd public socket address does not match the configured HTTP listener",
        ));
    }
    tokio::net::TcpListener::from_std(listener)
}
