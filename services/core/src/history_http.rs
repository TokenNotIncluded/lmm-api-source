//! HTTP routing and contract handlers for historical logs and usage exports.
//! Retained historical logs/exports and read-only native dashboard projections.

mod dashboard;

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    finance_export::{self, ExportFormat},
    history_read::{HistoryError, HistoryQuery, HistoryStore, ReadRoute},
};

const JS_SAFE: i64 = 9_007_199_254_740_991;

// Flatten buffers URL-encoded numbers as strings and loses unknown-field checks.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    p: Option<u32>,
    page_size: Option<u32>,
    r#type: Option<i16>,
    start_timestamp: Option<i64>,
    end_timestamp: Option<i64>,
    username: Option<String>,
    token_name: Option<String>,
    model_name: Option<String>,
    channel: Option<i64>,
    group: Option<String>,
    request_id: Option<String>,
    upstream_request_id: Option<String>,
    format: Option<String>,
}

impl ExportParams {
    fn into_history_query(self) -> HistoryQuery {
        HistoryQuery {
            p: self.p,
            page_size: self.page_size,
            r#type: self.r#type,
            start_timestamp: self.start_timestamp,
            end_timestamp: self.end_timestamp,
            username: self.username,
            token_name: self.token_name,
            model_name: self.model_name,
            channel: self.channel,
            group: self.group,
            request_id: self.request_id,
            upstream_request_id: self.upstream_request_id,
        }
    }
}

pub fn router(store: HistoryStore) -> Router {
    Router::new()
        .route("/api/log", get(logs))
        .route("/api/log/", get(logs))
        .route("/api/log/stat", get(stats))
        .route("/api/log/token", get(token_logs))
        .route("/api/finance/export", get(export_usage))
        .route("/api/finance/site-statistics", get(site_statistics))
        .route("/api/perf-metrics/summary", get(performance_summary))
        .route("/api/perf-metrics", get(performance_detail))
        .route("/api/data/flow", get(dashboard::flow))
        .route("/api/data/flow/self", get(dashboard::self_flow))
        .route("/api/data/users", get(dashboard::users))
        .route("/api/mj", get(drawing_admin))
        .route("/api/mj/", get(drawing_admin))
        .route("/api/mj/self", get(drawing_self))
        .route("/api/mj/self/", get(drawing_self))
        .route("/api/task", get(task_admin))
        .route("/api/task/", get(task_admin))
        .route("/api/task/self", get(task_self))
        .route("/api/task/self/", get(task_self))
        .with_state(store)
        .layer(middleware::from_fn(no_store))
}

async fn site_statistics(State(store): State<HistoryStore>, headers: HeaderMap) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(secret) => secret,
        Err(error) => return error_response(error),
    };
    match store.site_statistics(secret).await {
        Ok(statistics) => Json(json!({"success":true,"data":statistics})).into_response(),
        Err(HistoryError::Unavailable) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "success": false,
                "code": "SITE_STATISTICS_SOURCE_UNAVAILABLE",
                "availability": "unavailable",
                "message": "Site statistics require complete verified credit totals and original payment evidence",
                "missing_sources": ["historical_credit_totals_with_conversion_basis",
                    "historical_top_ups_with_original_currency_paid_and_refunded_amounts"]
            })),
        ).into_response(),
        Err(error) => error_response(error),
    }
}

async fn performance_summary(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<crate::history_read::performance::Query>, QueryRejection>,
) -> Response {
    performance_read(store, headers, query, false).await
}

async fn performance_detail(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<crate::history_read::performance::Query>, QueryRejection>,
) -> Response {
    performance_read(store, headers, query, true).await
}

async fn performance_read(
    store: HistoryStore,
    headers: HeaderMap,
    query: Result<Query<crate::history_read::performance::Query>, QueryRejection>,
    detail: bool,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(v) => v,
        Err(e) => return error_response(e),
    };
    let Query(query) = match query {
        Ok(v) => v,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    match store.performance(secret, &query, detail).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(e),
    }
}

async fn unavailable_category(
    store: HistoryStore,
    headers: HeaderMap,
    route: ReadRoute,
    category: &'static str,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(secret) => secret,
        Err(error) => return error_response(error),
    };
    if let Err(error) = store.authenticate_category(secret, route).await {
        return error_response(error);
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "success": false,
            "code": "USAGE_HISTORY_UNAVAILABLE",
            "availability": "unavailable",
            "category": category,
            "message": "This usage history category is unavailable"
        })),
    )
        .into_response()
}

async fn drawing_admin(State(store): State<HistoryStore>, headers: HeaderMap) -> Response {
    unavailable_category(store, headers, ReadRoute::Administrator, "drawing").await
}
async fn drawing_self(State(store): State<HistoryStore>, headers: HeaderMap) -> Response {
    unavailable_category(store, headers, ReadRoute::SelfSession, "drawing").await
}
async fn task_admin(State(store): State<HistoryStore>, headers: HeaderMap) -> Response {
    unavailable_category(store, headers, ReadRoute::Administrator, "task").await
}
async fn task_self(State(store): State<HistoryStore>, headers: HeaderMap) -> Response {
    unavailable_category(store, headers, ReadRoute::SelfSession, "task").await
}

async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = if request.uri().query().is_some_and(|q| q.len() > 4096) {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "code": "HISTORY_INVALID",
                "message": "History query exceeds 4096 bytes"
            })),
        )
            .into_response()
    } else {
        next.run(request).await
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn extract_bearer(headers: &HeaderMap) -> Result<&str, HistoryError> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(HistoryError::Unauthorized);
    }
    let (scheme, secret) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .ok_or(HistoryError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(HistoryError::Unauthorized);
    }
    Ok(secret)
}

fn error_response(err: HistoryError) -> Response {
    let (status, code, message) = match err {
        HistoryError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "AUTH_UNAUTHORIZED",
            "Authentication is required",
        ),
        HistoryError::Forbidden => (
            StatusCode::FORBIDDEN,
            "HISTORY_FORBIDDEN",
            "Administrative or appropriate token permissions are required",
        ),
        HistoryError::Invalid => (
            StatusCode::BAD_REQUEST,
            "HISTORY_INVALID",
            "Invalid history query parameters, pagination or time window",
        ),
        HistoryError::TooDense => (
            StatusCode::BAD_REQUEST,
            "HISTORY_TOO_DENSE",
            "Query result or export size limit exceeded",
        ),
        HistoryError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "HISTORY_UNAVAILABLE",
            "Historical data source is not installed or incomplete",
        ),
    };
    (
        status,
        Json(json!({
            "success": false,
            "code": code,
            "message": message
        })),
    )
        .into_response()
}

async fn logs(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    let Query(q) = match query {
        Ok(q) => q,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    match store.read(secret, ReadRoute::Administrator, &q).await {
        Ok(snapshot) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "",
                "data": snapshot
            })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

async fn stats(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    let Query(mut q) = match query {
        Ok(q) => q,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    q.p = Some(1);
    q.page_size = Some(1);
    match store.read(secret, ReadRoute::Administrator, &q).await {
        Ok(snapshot) => {
            // Raw string is authoritative and never lossy.
            let raw_str = &snapshot.summary.raw_consumption_quota;
            // Numeric quota compatibility field: only populated if nonnegative and within JS-safe integer range.
            // If it exceeds JS_SAFE, overflows i64, or is empty/invalid, it is strictly null (never coerced to 0).
            let quota_numeric = raw_str
                .parse::<i64>()
                .ok()
                .filter(|&v| (0..=JS_SAFE).contains(&v));
            let numeric_representable = quota_numeric.is_some();

            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "",
                    "data": {
                        "quota": quota_numeric,
                        "quota_numeric_representable": numeric_representable,
                        "raw_consumption_quota": raw_str,
                        "invalid_quota_records": snapshot.summary.invalid_quota_records,
                        "unpriced_quota_records": snapshot.summary.unpriced_quota_records,
                        "quota_projection_truncated": snapshot.summary.quota_projection_truncated,
                        "quota_usd": snapshot.summary.money.quota_usd,
                        "quota_money_available": snapshot.summary.money.quota_money_available,
                        "quota_money_basis": snapshot.summary.money.quota_money_basis,
                        "rpm": null,
                        "tpm": null,
                        "rates_available": false,
                        "source": snapshot.source,
                    }
                })),
            )
                .into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn token_logs(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    let Query(q) = match query {
        Ok(q) => q,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    match store.read(secret, ReadRoute::ReadOnlyKey, &q).await {
        Ok(snapshot) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "",
                "data": snapshot
            })),
        )
            .into_response(),
        Err(e) => error_response(e),
    }
}

async fn export_usage(
    State(store): State<HistoryStore>,
    headers: HeaderMap,
    query: Result<Query<ExportParams>, QueryRejection>,
) -> Response {
    let secret = match extract_bearer(&headers) {
        Ok(s) => s,
        Err(e) => return error_response(e),
    };
    let Query(params) = match query {
        Ok(p) => p,
        Err(_) => return error_response(HistoryError::Invalid),
    };
    let format = match params.format.as_deref() {
        Some("csv") => ExportFormat::Csv,
        Some("json") => ExportFormat::Json,
        Some(_) => return error_response(HistoryError::Invalid),
        None => {
            if headers
                .get(header::ACCEPT)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|a| a.contains("text/csv"))
            {
                ExportFormat::Csv
            } else {
                ExportFormat::Json
            }
        }
    };
    let query = params.into_history_query();
    match finance_export::export(&store, secret, &query, format).await {
        Ok(doc) => {
            let mut resp = doc.bytes.into_response();
            let headers_mut = resp.headers_mut();
            headers_mut.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(doc.content_type),
            );
            let disposition = format!("attachment; filename=\"{}\"", doc.filename);
            if let Ok(val) = header::HeaderValue::from_str(&disposition) {
                headers_mut.insert(header::CONTENT_DISPOSITION, val);
            }
            headers_mut.insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            );
            resp
        }
        Err(e) => error_response(e),
    }
}

#[cfg(test)]
#[path = "history_http/statistics_tests.rs"]
mod statistics_tests;

#[cfg(test)]
mod export_query_tests {
    use super::*;

    #[test]
    fn export_query_preserves_all_filters() {
        let filters = concat!(
            "p=2&page_size=25&type=2&start_timestamp=1700000001&",
            "end_timestamp=1700000003&username=alice&token_name=key-two&",
            "model_name=gpt-4&channel=2&group=default&request_id=req-2&",
            "upstream_request_id=up-2"
        );
        for (format, prefix) in [(Some("csv"), "format=csv&"), (None, "")] {
            let uri = format!("/api/finance/export?{prefix}{filters}")
                .parse()
                .unwrap();
            let Query(params) = Query::<ExportParams>::try_from_uri(&uri).unwrap();
            assert_eq!(params.format.as_deref(), format);
            let query = params.into_history_query();
            // Preserve paging so the store can reject paginated exports.
            assert_eq!(query.p, Some(2));
            assert_eq!(query.page_size, Some(25));
            assert_eq!(query.r#type, Some(2));
            assert_eq!(query.start_timestamp, Some(1700000001));
            assert_eq!(query.end_timestamp, Some(1700000003));
            assert_eq!(query.username.as_deref(), Some("alice"));
            assert_eq!(query.token_name.as_deref(), Some("key-two"));
            assert_eq!(query.model_name.as_deref(), Some("gpt-4"));
            assert_eq!(query.channel, Some(2));
            assert_eq!(query.group.as_deref(), Some("default"));
            assert_eq!(query.request_id.as_deref(), Some("req-2"));
            assert_eq!(query.upstream_request_id.as_deref(), Some("up-2"));
        }
    }

    #[test]
    fn export_query_rejects_unknown_duplicate_and_invalid_fields() {
        for query in [
            "format=csv&unknown=1",
            "format=csv&format=json",
            "type=2&type=3",
            "type=2&%74ype=3",
            "username=alice&username=bob",
            "type=32768",
            "channel=9223372036854775808",
            "start_timestamp=not-a-number",
        ] {
            let uri = format!("/api/finance/export?{query}").parse().unwrap();
            assert!(
                Query::<ExportParams>::try_from_uri(&uri).is_err(),
                "{query}"
            );
        }
    }
}
