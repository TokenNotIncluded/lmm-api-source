// This is the Go extension's private native-plugin worker, not a public server.
// Native plugin code is trusted code; process separation is not an OS sandbox.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
compile_error!("Extore native hosting currently requires Linux GNU");

mod args;
mod http;
mod paths;

use extore_plugin_api::{
    CONFIG_SCHEMA_VERSION, CommerceApi, Config, MAX_CONFIG_BYTES, MAX_RESPONSE_BYTES,
};
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{cell::RefCell, convert::Infallible, future::ready, process::ExitCode, time::Duration};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};
use zplugin::{Plugin, UnloadPolicy};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Messages are fixed labels, never paths, credentials or plugin output.
            eprintln!("Extore plugin host: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), &'static str> {
    let args = args::parse(std::env::args_os().skip(1))?;
    let owner = paths::private_directory(&args.data_dir)?;
    let socket_owner =
        paths::private_directory(args.socket.parent().ok_or("invalid socket path")?)?;
    if owner != socket_owner {
        return Err("Extore data and socket owners differ");
    }
    paths::verify_library(&args.library, &args.sha256, owner)?;
    if std::fs::symlink_metadata(&args.socket).is_ok() {
        return Err("plugin socket already exists");
    }
    // SAFETY: the operator explicitly pins a trusted native artifact and SHA256.
    // Go checks owner-controlled paths before starting this worker. All objects,
    // calls and returned buffers stay on this one OS thread until destruction.
    let library = unsafe { Plugin::load_path(&args.library, UnloadPolicy::Pinned) }
        .map_err(|_| "plugin load or ABI validation failed")?;
    let commerce =
        RefCell::new(CommerceApi::create(&library).map_err(|_| "commerce interface unavailable")?);
    let config = serde_json::to_vec(&Config {
        schema_version: CONFIG_SCHEMA_VERSION,
        data_dir: args.data_dir,
        origin: args.origin.clone(),
    })
    .map_err(|_| "plugin configuration invalid")?;
    if config.len() > MAX_CONFIG_BYTES {
        return Err("plugin configuration too large");
    }
    commerce
        .borrow_mut()
        .configure(&config)
        .map_err(|_| "plugin configuration failed")?;

    let mut terminate =
        signal(SignalKind::terminate()).map_err(|_| "signal registration failed")?;
    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(|_| "signal registration failed")?;
    let listener = UnixListener::bind(&args.socket).map_err(|_| "could not bind private socket")?;
    let _socket = paths::SocketGuard::new(&args.socket)?;
    let catalog = |token: &str| {
        let mut instance = commerce.borrow_mut();
        let buffer = instance.catalog(token).map_err(|_| ())?;
        if buffer.len() > MAX_RESPONSE_BYTES {
            return Err(());
        }
        // Deserialize before Buffer drops, releasing plugin-owned memory while
        // this instance and its pinned library are still alive on this thread.
        serde_json::from_slice(&buffer).map_err(|_| ())
    };
    loop {
        let (stream, _) = tokio::select! {
            _ = terminate.recv() => return Ok(()),
            _ = interrupt.recv() => return Ok(()),
            accepted = listener.accept() => accepted.map_err(|_| "private socket accept failed")?,
        };
        // Catalog reads are low frequency. One bounded connection at a time
        // avoids moving zplugin's non-Send/non-Sync proxies or adding a queue.
        let service = service_fn(|request| {
            ready(Ok::<_, Infallible>(http::handle(
                request,
                &args.origin,
                &catalog,
            )))
        });
        let mut builder = http1::Builder::new();
        builder
            .keep_alive(false)
            .max_headers(16)
            .max_buf_size(16 * 1024)
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(5));
        let connection = builder.serve_connection(TokioIo::new(stream), service);
        tokio::select! {
            _ = terminate.recv() => return Ok(()),
            _ = interrupt.recv() => return Ok(()),
            _ = tokio::time::timeout(Duration::from_secs(20), connection) => {}
        }
    }
}
