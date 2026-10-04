use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct EndpointState {
    origin: HeaderValue,
    rate: Arc<Mutex<(Instant, u32)>>,
}

async fn measurement(State(state): State<EndpointState>, headers: HeaderMap) -> impl IntoResponse {
    // Same-origin GET requests can omit Origin; a fixed empty response has no private data.
    if headers
        .get("origin")
        .is_some_and(|origin| origin != state.origin)
    {
        return (StatusCode::FORBIDDEN, HeaderMap::new());
    }
    let accepted = match state.rate.lock() {
        Ok(mut rate) => {
            if rate.0.elapsed() >= Duration::from_secs(1) {
                *rate = (Instant::now(), 0);
            }
            rate.1 += 1;
            rate.1 <= 100
        }
        Err(_) => false,
    };
    let mut result = HeaderMap::new();
    result.insert("access-control-allow-origin", state.origin);
    result.insert(
        "cache-control",
        HeaderValue::from_static("no-store, max-age=0"),
    );
    result.insert("vary", HeaderValue::from_static("Origin"));
    result.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    (
        if accepted {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::TOO_MANY_REQUESTS
        },
        result,
    )
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    origin: String,
    shutdown: CancellationToken,
) -> Result<()> {
    let state = EndpointState {
        origin: origin.parse()?,
        rate: Arc::new(Mutex::new((Instant::now(), 0))),
    };
    let app = Router::new()
        .route("/pinglake/latency", get(measurement))
        .with_state(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fixed_endpoint_enforces_origin_and_never_exposes_credentials() {
        let state = EndpointState {
            origin: HeaderValue::from_static("https://monitor.example.com"),
            rate: Arc::new(Mutex::new((Instant::now(), 0))),
        };
        let mut foreign = HeaderMap::new();
        foreign.insert(
            "origin",
            HeaderValue::from_static("https://other.example.com"),
        );
        let denied = measurement(State(state.clone()), foreign)
            .await
            .into_response();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let mut headers = HeaderMap::new();
        headers.insert("origin", state.origin.clone());
        let response = measurement(State(state), headers).await.into_response();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()["cache-control"], "no-store, max-age=0");
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-credentials")
        );
    }
    #[tokio::test]
    async fn same_origin_request_without_origin_can_measure() {
        let state = EndpointState {
            origin: HeaderValue::from_static("https://monitor.example.com"),
            rate: Arc::new(Mutex::new((Instant::now(), 0))),
        };
        assert_eq!(
            measurement(State(state), HeaderMap::new())
                .await
                .into_response()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
}
