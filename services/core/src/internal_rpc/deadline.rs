//! Keep the existing two-second RPC budget; only Siteverify gets twelve seconds.
use super::DEADLINE;
use axum::http::{Request, Response};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tonic::{Status, body::Body};
use tower::Service;

fn budget(path: &str) -> Duration {
    if path == "/lmm.core.v1.CoreCheckinLedger/Authorize" {
        Duration::from_secs(12)
    } else {
        DEADLINE
    }
}
#[derive(Clone)]
pub(super) struct Deadline<S>(pub S);
impl<S> Service<Request<Body>> for Deadline<S>
where
    S: Service<Request<Body>, Response = Response<Body>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }
    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let limit = budget(request.uri().path());
        let call = self.0.call(request);
        Box::pin(async move {
            match tokio::time::timeout(limit, call).await {
                Ok(result) => result,
                Err(_) => Ok(Status::deadline_exceeded("RPC deadline exceeded").into_http()),
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_checkin_authorize_extends_the_existing_budget() {
        assert_eq!(
            budget("/lmm.core.v1.CoreCheckinLedger/Authorize"),
            Duration::from_secs(12)
        );
        for path in [
            "/lmm.core.v1.CoreCheckinLedger/Post",
            "/lmm.core.v1.CoreCheckinLedger/Challenge",
            "/lmm.core.v1.CoreControl/Authorize",
            "/unknown",
        ] {
            assert_eq!(budget(path), Duration::from_secs(2));
        }
    }
    #[tokio::test]
    async fn middleware_enforces_existing_deadline() {
        let service = tower::service_fn(|_: Request<Body>| async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
        });
        let response = Deadline(service)
            .call(Request::new(Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.headers()["grpc-status"], "4");
    }
}
