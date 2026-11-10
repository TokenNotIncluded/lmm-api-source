use super::*;
use crate::payments::PaymentsStore;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::get,
};
#[derive(Clone)]
struct StateData {
    store: SubscriptionStore,
    payments: PaymentsStore,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavePrice {
    operation_key: String,
    price: crate::payments::subscription_pancake::PancakePrice,
}
pub(super) fn router(store: SubscriptionStore, payments: PaymentsStore) -> Router {
    Router::new()
        .route(
            "/core/v1/subscriptions/plans/{id}/pancake-prices",
            get(public_prices),
        )
        .route(
            "/core/v1/subscriptions/admin/plans/{id}/pancake-prices",
            get(admin_prices).post(save),
        )
        .layer(DefaultBodyLimit::max(16384))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    "private, no-store".parse().unwrap(),
                );
                response
                    .headers_mut()
                    .insert(axum::http::header::VARY, "Authorization".parse().unwrap());
                response
            },
        ))
        .with_state(StateData { store, payments })
}
async fn public_prices(
    State(s): State<StateData>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"prices":s.store.pancake_prices(super::http::bearer(&h)?,id,false).await?}),
    ))
}
async fn admin_prices(
    State(s): State<StateData>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"prices":s.store.pancake_prices(super::http::bearer(&h)?,id,true).await?}),
    ))
}
async fn save(
    State(s): State<StateData>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<SavePrice>,
) -> Result<Json<crate::payments::subscription_pancake::PancakePrice>> {
    if id != body.price.plan_id {
        return Err(Error::Invalid);
    }
    Ok(Json(
        s.store
            .save_pancake_price(
                &s.payments,
                super::http::bearer(&h)?,
                &body.operation_key,
                &body.price,
            )
            .await?,
    ))
}
