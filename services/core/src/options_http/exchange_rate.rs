//! Root-only reference quotes from a fixed public HTTPS provider.
//! Fetching a quote never persists an option or changes a ledger/payment price.

use super::{OptionsHttp, auth_error, authorize, response};
use axum::{
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{NaiveDate, SecondsFormat, Utc};
use reqwest::{Client, ClientBuilder, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};

const PROVIDER_URL: &str = "https://api.frankfurter.dev/v1/latest";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

fn http_client() -> ClientBuilder {
    crate::http_client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
}

#[derive(Clone)]
pub(super) struct Reader {
    client: Option<Client>,
    endpoint: Url,
}

impl Reader {
    pub(super) fn new() -> Self {
        Self {
            client: http_client().build().ok(),
            endpoint: Url::parse(PROVIDER_URL).expect("fixed exchange-rate URL"),
        }
    }

    async fn fetch(&self, currency: &str) -> Result<Quote, ()> {
        let now = Utc::now();
        if currency == "USD" {
            return Ok(Quote {
                base_currency: "USD",
                quote_currency: currency.into(),
                rate: 1.0,
                fetched_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                rate_date: now.date_naive().to_string(),
                provider: "base",
            });
        }
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .append_pair("base", "USD")
            .append_pair("symbols", currency);
        let mut result = self
            .client
            .as_ref()
            .ok_or(())?
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| ())?;
        if !result.status().is_success()
            || result
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(());
        }
        let mut body = Vec::new();
        while let Some(chunk) = result.chunk().await.map_err(|_| ())? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(());
            }
            body.extend_from_slice(&chunk);
        }
        let data: ProviderQuote = serde_json::from_slice(&body).map_err(|_| ())?;
        let rate = *data.rates.get(currency).ok_or(())?;
        let date = NaiveDate::parse_from_str(&data.date, "%Y-%m-%d").map_err(|_| ())?;
        if data.base != "USD"
            || data.amount != 1.0
            || data.date.len() != 10
            || date > Utc::now().date_naive()
            || !rate.is_finite()
            || rate <= 0.0
            || rate > 1e9
        {
            return Err(());
        }
        Ok(Quote {
            base_currency: "USD",
            quote_currency: currency.into(),
            rate,
            fetched_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            rate_date: data.date,
            provider: "frankfurter.dev",
        })
    }
}

#[derive(Deserialize)]
struct ProviderQuote {
    amount: f64,
    base: String,
    date: String,
    rates: BTreeMap<String, f64>,
}

#[derive(Serialize)]
struct Quote {
    base_currency: &'static str,
    quote_currency: String,
    rate: f64,
    fetched_at: String,
    rate_date: String,
    provider: &'static str,
}

fn currency(query: Option<&str>) -> Result<String, ()> {
    let mut fields = form_urlencoded::parse(query.ok_or(())?.as_bytes());
    let (name, value) = fields.next().ok_or(())?;
    if name != "currency" || fields.next().is_some() {
        return Err(());
    }
    let currency = value.trim().to_ascii_uppercase();
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(());
    }
    Ok(currency)
}

pub(super) async fn read(
    State(state): State<OptionsHttp>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    if let Err(error) = authorize(&state, &headers).await {
        return auth_error(error);
    }
    let Ok(currency) = currency(query.as_deref()) else {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"success":false,"code":"EXCHANGE_RATE_INVALID_CURRENCY",
                "message":"currency must be a three-letter ISO code"}),
        );
    };
    match state.exchange_rates.fetch(&currency).await {
        Ok(quote) => response(
            StatusCode::OK,
            json!({"success":true,"message":"","data":quote}),
        ),
        Err(()) => response(
            StatusCode::BAD_GATEWAY,
            json!({"success":false,"code":"EXCHANGE_RATE_UNAVAILABLE",
                "message":"The reference exchange rate could not be loaded"}),
        ),
    }
}

#[cfg(test)]
mod tests;
