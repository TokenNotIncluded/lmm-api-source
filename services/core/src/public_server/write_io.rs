//! Bounded public socket writes independent of request or stream lifetimes.
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// Maximum duration of a pending downstream write operation.
pub const PUBLIC_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

// Write timeout semantics adapted from the public Tokio adapter tokio-io-timeout
// 1.2.1, sfackler/tokio-io-timeout@c2406c689c782e9005af376b7359007a4a8faa96,
// src/lib.rs TimeoutState and TimeoutWriter. Kept local to avoid adding a new
// dependency; boxed Sleep replaces upstream pin-project-lite projection.
// Copyright (c) 2017 The tokio-io-timeout Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
/// Tokio connection adapter: only pending writes arm its progress timer.
/// Reads and idle/healthy streams have no connection-wide lifetime timeout.
pub struct PublicWriteIo<S> {
    inner: S,
    pending: Option<Pin<Box<tokio::time::Sleep>>>,
    timeout_abort: Option<fn(&S) -> io::Result<()>>,
    failed: Option<(io::ErrorKind, Option<i32>)>,
}
impl<S> PublicWriteIo<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            pending: None,
            timeout_abort: None,
            failed: None,
        }
    }
    fn with_timeout_abort(mut self, abort: fn(&S) -> io::Result<()>) -> Self {
        self.timeout_abort = Some(abort);
        self
    }
    fn failure(&self) -> Option<io::Error> {
        self.failed
            .map(|(kind, errno)| errno.map_or_else(|| kind.into(), io::Error::from_raw_os_error))
    }
    fn checked<T>(
        &mut self,
        cx: &mut Context<'_>,
        result: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        use std::future::Future;
        match result {
            Poll::Pending => {
                let timer = self
                    .pending
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(PUBLIC_WRITE_TIMEOUT)));
                if timer.as_mut().poll(cx).is_ready() {
                    self.pending = None;
                    self.failed = Some((io::ErrorKind::TimedOut, None));
                    // Abort only this already failed transport. Normal FIN and
                    // healthy shared HTTP/2 connections keep their original behavior.
                    // Tokio 1.53.2 set_zero_linger is nonblocking; positive linger is not used.
                    // @mark model.socket-pressure role=implementation
                    if let Some(abort) = self.timeout_abort
                        && let Err(error) = abort(&self.inner)
                    {
                        self.failed = Some((error.kind(), error.raw_os_error()));
                        // Payload-free fields only: no peer, request, raw error description or credentials.
                        eprintln!(
                            "core_socket_abort_failed kind={:?} os_error={:?}",
                            error.kind(),
                            error.raw_os_error()
                        );
                        return Poll::Ready(Err(error));
                    }
                    Poll::Ready(Err(io::ErrorKind::TimedOut.into()))
                } else {
                    Poll::Pending
                }
            }
            ready => {
                self.pending = None;
                ready
            }
        }
    }
}
impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for PublicWriteIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(error) = self.failure() {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for PublicWriteIo<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(error) = self.failure() {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.checked(cx, result)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if let Some(error) = self.failure() {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.checked(cx, result)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(error) = self.failure() {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        self.checked(cx, result)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(error) = self.failure() {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.checked(cx, result)
    }
}
impl PublicWriteIo<tokio::net::TcpStream> {
    /// Drop kernel buffers only for rejected transports or connections that
    /// never reached a parsed request. Healthy streams retain normal closing.
    pub(crate) fn reject_unadmitted(&self) -> io::Result<()> {
        self.inner.set_zero_linger()
    }
}

/// Axum listener seam shared by the executable and exported integration fixtures.
pub struct PublicWriteListener(tokio::net::TcpListener);
impl axum::serve::Listener for PublicWriteListener {
    type Io = PublicWriteIo<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, addr) = axum::serve::Listener::accept(&mut self.0).await;
        (
            PublicWriteIo::new(stream).with_timeout_abort(tokio::net::TcpStream::set_zero_linger),
            addr,
        )
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.0.local_addr()
    }
}
// Axum's TapIo has the upstream Connected<IncomingStream> implementation for
// SocketAddr; retain existing ConnectInfo extractors without an orphan impl.
pub type PublicListener =
    axum::serve::TapIo<PublicWriteListener, fn(&mut PublicWriteIo<tokio::net::TcpStream>)>;
pub fn public_listener(listener: tokio::net::TcpListener) -> PublicListener {
    use axum::serve::ListenerExt;
    PublicWriteListener(listener).tap_io((|_| {}) as fn(&mut PublicWriteIo<tokio::net::TcpStream>))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, Bytes},
        http::header,
    };
    use tokio::sync::watch;
    use tokio_stream::StreamExt;

    struct PendingWriter;
    impl tokio::io::AsyncWrite for PendingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
        fn poll_write_vectored(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[io::IoSlice<'_>],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }
    #[tokio::test]
    async fn public_write_timeout_bounds_each_pending_operation() {
        use tokio::io::AsyncWriteExt;
        async fn pending(mode: usize) {
            let mut writer = PublicWriteIo::new(PendingWriter);
            let result =
                tokio::time::timeout(PUBLIC_WRITE_TIMEOUT + Duration::from_secs(1), async {
                    match mode {
                        0 => writer.write(b"x").await.map(|_| ()),
                        1 => writer
                            .write_vectored(&[io::IoSlice::new(b"x")])
                            .await
                            .map(|_| ()),
                        2 => writer.flush().await,
                        _ => writer.shutdown().await,
                    }
                })
                .await
                .expect("pending operation did not wake");
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        }
        tokio::join!(pending(0), pending(1), pending(2), pending(3));
    }
    #[derive(Default)]
    struct AbortProbe {
        polls: std::sync::atomic::AtomicUsize,
        aborts: std::sync::atomic::AtomicUsize,
        ready: std::sync::atomic::AtomicBool,
        abort_fails: bool,
    }
    impl AbortProbe {
        fn poll(&self) -> Poll<io::Result<usize>> {
            use std::sync::atomic::Ordering;
            self.polls.fetch_add(1, Ordering::SeqCst);
            if self.ready.load(Ordering::SeqCst) {
                Poll::Ready(Ok(1))
            } else {
                Poll::Pending
            }
        }
        fn abort(&self) -> io::Result<()> {
            self.aborts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.abort_fails {
                Err(io::Error::from_raw_os_error(13))
            } else {
                Ok(())
            }
        }
    }
    impl tokio::io::AsyncRead for AbortProbe {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.poll().map(|result| result.map(|_| ()))
        }
    }
    impl tokio::io::AsyncWrite for AbortProbe {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.poll()
        }
        fn poll_write_vectored(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[io::IoSlice<'_>],
        ) -> Poll<io::Result<usize>> {
            self.poll()
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll().map(|result| result.map(|_| ()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll().map(|result| result.map(|_| ()))
        }
    }
    async fn sticky_abort_case(mode: usize, abort_fails: bool) {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = PublicWriteIo::new(AbortProbe {
            abort_fails,
            ..Default::default()
        })
        .with_timeout_abort(AbortProbe::abort);
        // Test the transition without sleeping five seconds in every matrix cell.
        // Existing public_write_timeout_bounds_each_pending_operation verifies the real budget.
        stream.pending = Some(Box::pin(tokio::time::sleep(Duration::ZERO)));
        let error = tokio::time::timeout(Duration::from_secs(1), async {
            match mode {
                0 => stream.write(b"x").await.map(|_| ()),
                1 => stream
                    .write_vectored(&[io::IoSlice::new(b"x")])
                    .await
                    .map(|_| ()),
                2 => stream.flush().await,
                _ => stream.shutdown().await,
            }
        })
        .await
        .unwrap()
        .unwrap_err();
        if abort_fails {
            assert_eq!(error.raw_os_error(), Some(13));
        } else {
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        }
        assert_eq!(stream.inner.aborts.load(Ordering::SeqCst), 1);
        let polls = stream.inner.polls.load(Ordering::SeqCst);
        stream.inner.ready.store(true, Ordering::SeqCst);
        let mut byte = [0u8; 1];
        for error in [
            stream.write(b"x").await.unwrap_err(),
            stream
                .write_vectored(&[io::IoSlice::new(b"x")])
                .await
                .unwrap_err(),
            stream.flush().await.unwrap_err(),
            stream.shutdown().await.unwrap_err(),
            stream.read(&mut byte).await.unwrap_err(),
        ] {
            if abort_fails {
                assert_eq!(error.raw_os_error(), Some(13));
            } else {
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            }
        }
        assert_eq!(
            stream.inner.polls.load(Ordering::SeqCst),
            polls,
            "failed socket must short circuit before touching inner IO"
        );
        assert_eq!(
            stream.inner.aborts.load(Ordering::SeqCst),
            1,
            "abort is invoked only once"
        );
    }
    #[tokio::test]
    async fn public_write_abort_is_sticky_for_every_pending_operation() {
        tokio::join!(
            sticky_abort_case(0, false),
            sticky_abort_case(1, false),
            sticky_abort_case(2, false),
            sticky_abort_case(3, false)
        );
    }
    #[tokio::test]
    async fn public_write_abort_failure_preserves_errno_and_prevents_more_io() {
        tokio::join!(
            sticky_abort_case(0, true),
            sticky_abort_case(1, true),
            sticky_abort_case(2, true),
            sticky_abort_case(3, true)
        );
    }
    #[tokio::test]
    async fn public_write_progress_never_invokes_abort() {
        use std::sync::atomic::Ordering;
        use tokio::io::AsyncWriteExt;
        let mut stream =
            PublicWriteIo::new(AbortProbe::default()).with_timeout_abort(AbortProbe::abort);
        stream.inner.ready.store(true, Ordering::SeqCst);
        assert_eq!(stream.write(b"x").await.unwrap(), 1);
        assert_eq!(
            stream
                .write_vectored(&[io::IoSlice::new(b"x")])
                .await
                .unwrap(),
            1
        );
        stream.flush().await.unwrap();
        stream.shutdown().await.unwrap();
        assert_eq!(stream.inner.aborts.load(Ordering::SeqCst), 0);
        assert!(stream.failed.is_none());
        assert!(stream.pending.is_none());
    }
    #[tokio::test]
    async fn public_listener_installs_abort_only_for_an_expired_write() {
        use axum::serve::Listener;
        use tokio::io::AsyncReadExt;
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let mut listener = PublicWriteListener(socket);
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (mut accepted, _) = listener.accept().await;
        assert_eq!(
            accepted.inner.linger().unwrap(),
            None,
            "normal socket must retain FIN semantics"
        );
        accepted.pending = Some(Box::pin(tokio::time::sleep(Duration::ZERO)));
        let result = std::future::poll_fn(|cx| accepted.checked::<()>(cx, Poll::Pending)).await;
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(accepted.inner.linger().unwrap(), Some(Duration::ZERO));
        drop(accepted);
        let mut byte = [0u8; 1];
        let error = tokio::time::timeout(Duration::from_secs(1), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    }

    #[tokio::test]
    async fn public_listener_keeps_progressing_stream_beyond_write_timeout() {
        use axum::routing::get;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/events",
            get(|| async {
                let chunks = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
                    Duration::from_secs(1),
                ))
                .take(8)
                .map(|_| Ok::<_, io::Error>(Bytes::from_static(b": healthy\n\n")));
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(chunks),
                )
            }),
        );
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            axum::serve(public_listener(listener), app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.changed().await;
                })
                .await
                .unwrap();
        });
        let response = crate::http_client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/events"))
            .send()
            .await
            .unwrap();
        let start = tokio::time::Instant::now();
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(2), chunks.next())
            .await
            .unwrap()
        {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        assert!(
            start.elapsed() > PUBLIC_WRITE_TIMEOUT,
            "test did not outlive pending-write budget"
        );
        assert_eq!(bytes, b": healthy\n\n".repeat(8));
        stop.send(true).unwrap();
        task.await.unwrap();
    }
}
