//! Real Core schema, identity, native keys and ledger, with a counting loopback
//! image supplier. No external provider request or handwritten schema.
use super::*;
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        image_price::{ExactRatio, ImageCountBasis, ImagePriceRule, ImageTokenUsageContract},
        ledger::{Action, Ledger, Outcome as LedgerOutcome, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    image::ImageMediaType,
    model_http::image_provider::{ImageMultipartInput, ImageUpload},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

#[derive(Clone, Copy)]
pub(super) enum Reply {
    OneImage,
    SlowOneImage,
    Empty,
    Truncated,
    NativeImage,
    NativeImageUnknownCache,
    NativeImageAbsentCache,
    NativeImageByPrompt,
}
fn native_image_response(unknown_cache: bool) -> Value {
    let mut usage = crate::billing::image_price::native_tokens::tests::measured();
    if unknown_cache {
        usage["input_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cached_tokens_details");
    }
    json!({"created":1,"data":[{"b64_json":"AQID"}],"usage":usage,"size":"1024x1024","quality":"standard"})
}
fn absent_cache_image_response() -> Value {
    let mut response = native_image_response(true);
    response["usage"]["input_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("cached_tokens");
    response
}
#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
    reply: Reply,
}
pub(super) struct Supplier {
    base_url: String,
    pub(super) hits: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
    task: JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn supplier_reply(
    State(state): State<SupplierState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers["authorization"],
        "Bearer synthetic-image-provider-key"
    );
    assert_eq!(input["model"], "supplier-image");
    assert_eq!(input["size"], "1024x1024");
    assert_eq!(input["quality"], "standard");
    if matches!(state.reply, Reply::SlowOneImage) {
        // Release only after the test observes dispatch and drain, eliminating
        // dependence on scheduler speed or an arbitrary response delay.
        state.release.acquire().await.unwrap().forget();
    }
    match state.reply {
        Reply::NativeImageAbsentCache => Json(absent_cache_image_response()).into_response(),
        Reply::NativeImageByPrompt => Json(native_image_response(input["prompt"] == "unknown-cache")).into_response(),
        Reply::NativeImage | Reply::NativeImageUnknownCache => Json(native_image_response(matches!(state.reply, Reply::NativeImageUnknownCache))).into_response(),
        // A completed, syntactically valid provider image payload is the image
        // usage evidence. The relay does not infer image cost from text tokens.
        Reply::OneImage | Reply::SlowOneImage => {
            Json(json!({"created":1,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII="}]})).into_response()
        }
        Reply::Empty => Json(json!({"created":1,"data":[]})).into_response(),
        Reply::Truncated => {
            let stream = tokio_stream::iter([
                Ok::<Bytes, std::io::Error>(Bytes::from_static(b"{\"created\":1,\"data\":[")),
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "synthetic truncated image body",
                )),
            ]);
            (
                [("content-type", "application/json")],
                Body::from_stream(stream),
            )
                .into_response()
        }
    }
}
async fn supplier_edit_reply(
    State(state): State<SupplierState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers["authorization"],
        "Bearer synthetic-image-provider-key"
    );
    let mut images = 0usize;
    let mut model = None;
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_owned();
        if matches!(name.as_str(), "image" | "image[]") {
            assert_eq!(field.content_type(), Some("image/png"));
            let bytes = field.bytes().await.unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            images += 1;
        } else if name == "model" {
            model = Some(field.text().await.unwrap());
        }
    }
    assert!(images > 0);
    assert_eq!(model.as_deref(), Some("supplier-image"));
    if matches!(state.reply, Reply::Truncated) {
        return (axum::http::StatusCode::OK, "{\"data\":[").into_response();
    }
    if matches!(
        state.reply,
        Reply::NativeImage | Reply::NativeImageUnknownCache
    ) {
        return Json(native_image_response(matches!(
            state.reply,
            Reply::NativeImageUnknownCache
        )))
        .into_response();
    }
    if matches!(state.reply, Reply::NativeImageAbsentCache) {
        return Json(absent_cache_image_response()).into_response();
    }
    Json(json!({"created":1,"data":[{"b64_json":"AQID"}]})).into_response()
}
impl Supplier {
    pub(super) async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let app = Router::new()
            .route("/v1/images/generations", post(supplier_reply))
            .route("/v1/images/edits", post(supplier_edit_reply))
            .with_state(SupplierState {
                hits: hits.clone(),
                release: release.clone(),
                reply,
            });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base_url,
            hits,
            release,
            task,
        }
    }
}
struct Fixture {
    pool: PgPool,
    ledger: Ledger,
    secret: String,
    key_id: i64,
    account: i64,
}
async fn fixture(pool: PgPool) -> Fixture {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let legacy_activation: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_identity.developer_access WHERE user_id=7)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!legacy_activation);
    assert!(identity.authorize(&key.secret).await.is_ok());
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-image-pg-test')")
        .execute(&pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    let credited = ledger
        .execute(&LedgerRequest {
            scope: "image-api-pg-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds in disposable PostgreSQL".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(10000).unwrap(),
                payment_reference: "synthetic:image-api-pg-test".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(credited, LedgerOutcome::Posted { .. }));
    Fixture {
        pool,
        ledger,
        secret: key.secret,
        key_id: key.id,
        account: actor.owner_account_id,
    }
}
fn image_service(f: &Fixture, supplier: &Supplier) -> ImageService {
    image_service_for_pool(&f.pool, supplier)
}
#[tokio::test]
async fn image_provider_source_binding_allows_multiple_models_without_invented_channel_ids() {
    // Lazy pool is never queried; this exercises configuration/credential binding.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://unused@127.0.0.1/unused")
        .unwrap();
    let make = |secret: &str| {
        let mut rules = Vec::new();
        let mut specs = Vec::new();
        for (model, key) in [
            ("gpt-image-2", "synthetic-source-key"),
            ("gpt-image-2.5-flare", secret),
        ] {
            rules.push(ImagePriceRule {
                upstream_id: 9,
                vendor: "openai".into(),
                billing_group: "allowed".into(),
                upstream_model: model.into(),
                semantic_model: model.into(),
                endpoint: ImageEndpoint::Generations,
                size: "1024x1024".into(),
                quality: "high".into(),
                usd_micros_per_image: ExactRatio {
                    numerator: 1_000_000,
                    denominator: 1,
                },
                group_multiplier: ExactRatio {
                    numerator: 1,
                    denominator: 1,
                },
                max_n: 1,
                count_basis: ImageCountBasis::ReturnedPayloads,
                max_budget: Credits::try_from(500000).unwrap(),
                source_revision: "synthetic-binding-only".into(),
            });
            specs.push(ProviderSpec {
                upstream_id: 9,
                vendor: "openai".into(),
                upstream_model: model.into(),
                base_url: "http://127.0.0.1:1/v1".into(),
                secret: key.into(),
                local_http: true,
            });
        }
        ImageService::new(
            pool.clone(),
            Charging::new(pool.clone(), Arc::new(TransactionalLedger)),
            ImagePriceConfig { revision: 1, rules },
            specs,
            &Limits::default(),
        )
    };
    assert!(make("synthetic-source-key").is_ok());
    assert!(matches!(make("different-source-key"), Err(Error::Invalid)));
    pool.close().await;
}
pub(super) fn image_service_for_pool(pool: &PgPool, supplier: &Supplier) -> ImageService {
    let rules = [ImageEndpoint::Generations, ImageEndpoint::Edits]
        .into_iter()
        .map(|endpoint| ImagePriceRule {
            upstream_id: 1,
            vendor: "openai".into(),
            billing_group: "allowed".into(),
            upstream_model: "supplier-image".into(),
            semantic_model: "dall-e-3".into(),
            endpoint,
            size: "1024x1024".into(),
            quality: "standard".into(),
            // 0.004 USD * 500000 credits/USD = 2000 credits per image.
            usd_micros_per_image: ExactRatio {
                numerator: 4000,
                denominator: 1,
            },
            group_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_n: 4,
            count_basis: ImageCountBasis::ReturnedPayloads,
            max_budget: Credits::try_from(10000).unwrap(),
            source_revision: "synthetic-image-tariff-v1".into(),
        })
        .collect();
    ImageService::new(
        pool.clone(),
        Charging::new(pool.clone(), Arc::new(TransactionalLedger)),
        ImagePriceConfig { revision: 1, rules },
        vec![ProviderSpec {
            upstream_id: 1,
            vendor: "openai".into(),
            upstream_model: "supplier-image".into(),
            base_url: supplier.base_url.clone(),
            secret: "synthetic-image-provider-key".into(),
            local_http: true,
        }],
        &Limits {
            total_timeout: Duration::from_secs(3),
            ..Limits::default()
        },
    )
    .unwrap()
}
pub(super) fn input(n: u32) -> DrawingInput {
    DrawingInput {
        prompt: "a synthetic image".into(),
        model: "dall-e-3".into(),
        group: String::new(),
        size: "1024x1024".into(),
        quality: "standard".into(),
        n,
    }
}
fn groups() -> Vec<String> {
    vec!["allowed".into()]
}
fn operation(key: &str) -> String {
    format!("image:{key}")
}
async fn no_charges(f: &Fixture) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 10000);
    assert_eq!(funds.reserved_units, 0);
}

fn native_image_service(f: &Fixture, supplier: &Supplier) -> ImageService {
    let mut service = image_service(f, supplier);
    let mut rules = service.prices.rules.clone();
    for rule in &mut rules {
        rule.semantic_model = "gpt-image-2".into();
        rule.usd_micros_per_image.numerator = 0;
        rule.count_basis = ImageCountBasis::NativeTokens(
            crate::billing::image_price::native_tokens::tests::tariff(),
        );
        rule.source_revision = "production-shape-synthetic-usage-not-live".into();
    }
    let prices = ImagePriceConfig { revision: 2, rules };
    prices.validate().unwrap();
    service.prices = Arc::new(prices);
    service
}

fn legacy_cache_image_service(f: &Fixture, supplier: &Supplier) -> ImageService {
    let mut service = native_image_service(f, supplier);
    let mut prices = (*service.prices).clone();
    prices.revision += 1;
    for rule in &mut prices.rules {
        if let ImageCountBasis::NativeTokens(tariff) = &mut rule.count_basis {
            tariff.usage_contract = ImageTokenUsageContract::Cc52OpenaiImagesCacheAbsenceV1;
        }
    }
    service.prices = Arc::new(prices);
    service
}

#[sqlx::test(migrations = false)]
async fn image_api_explicit_legacy_cache_absence_keeps_raw_and_old_strict_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::NativeImageAbsentCache).await;
    let strict = native_image_service(&f, &supplier);
    let mut request = input(1);
    request.model = "gpt-image-2".into();
    let strict_key = format!("{:064x}", 1201);
    assert!(
        strict
            .execute_api(
                &f.secret,
                &strict_key,
                &groups(),
                request.clone(),
                None,
                None
            )
            .await
            .is_err()
    );
    let strict_price: Value =
        sqlx::query_scalar("SELECT price FROM core_image.operations WHERE id=$1")
            .bind(operation(&strict_key))
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(!strict_price.to_string().contains("usage_contract"));

    let legacy = legacy_cache_image_service(&f, &supplier);
    let key = format!("{:064x}", 1202);
    let receipt = legacy
        .execute_api(&f.secret, &key, &groups(), request, None, None)
        .await
        .unwrap();
    assert_eq!(receipt.charged_credits, Some(490));
    let row = sqlx::query("SELECT output,image_usage,price FROM core_image.operations WHERE id=$1")
        .bind(operation(&key))
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let raw: Value = row.try_get("output").unwrap();
    assert_eq!(raw["usage"], absent_cache_image_response()["usage"]);
    let usage: ImageUsage = serde_json::from_value(row.try_get("image_usage").unwrap()).unwrap();
    let tokens = usage.native_tokens.unwrap();
    assert_eq!(
        (tokens.cached, tokens.cached_text, tokens.cached_image),
        (Some(0), Some(0), Some(0))
    );
    let frozen: Value = row.try_get("price").unwrap();
    assert!(
        frozen
            .to_string()
            .contains("cc52_openai_images_cache_absence_v1")
    );

    let restarted = legacy_cache_image_service(&f, &supplier);
    assert_eq!(restarted.recover_receipts(16).await.unwrap(), 0);
    let unchanged: Value =
        sqlx::query_scalar("SELECT price FROM core_image.operations WHERE id=$1")
            .bind(operation(&strict_key))
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(unchanged, strict_price);
    let held = restarted
        .charging
        .resolve(&operation(&strict_key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((held.state.as_str(), held.reserved), ("reconcile", 1900));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((funds.available_units, funds.reserved_units), (7610, 1900));
}

#[sqlx::test(migrations = false)]
async fn image_api_legacy_cache_contract_keeps_positive_unknown_pending_and_present_exact(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let unknown = Supplier::start(Reply::NativeImageUnknownCache).await;
    let legacy = legacy_cache_image_service(&f, &unknown);
    let mut request = input(1);
    request.model = "gpt-image-2".into();
    let key = format!("{:064x}", 1203);
    assert!(
        legacy
            .execute_api(&f.secret, &key, &groups(), request.clone(), None, None)
            .await
            .is_err()
    );
    assert_eq!(legacy.recover_receipts(16).await.unwrap(), 0);
    let held = legacy
        .charging
        .resolve(&operation(&key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((held.state.as_str(), held.reserved), ("reconcile", 1900));
    let present = Supplier::start(Reply::NativeImage).await;
    let service = legacy_cache_image_service(&f, &present);
    let receipt = service
        .execute_api(
            &f.secret,
            &format!("{:064x}", 1204),
            &groups(),
            request,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(receipt.charged_credits, Some(381));
    assert_eq!(unknown.hits.load(Ordering::SeqCst), 1);
    assert_eq!(present.hits.load(Ordering::SeqCst), 1);
}

async fn block_image_completion(pool: &PgPool, enabled: bool) {
    if enabled {
        sqlx::raw_sql("CREATE FUNCTION public.fixture_image_complete_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.state='completed' THEN RAISE EXCEPTION 'synthetic image completion transaction failure' USING ERRCODE='40001'; END IF; RETURN NEW; END $$; CREATE TRIGGER fixture_image_complete_failure BEFORE UPDATE ON core_image.operations FOR EACH ROW EXECUTE FUNCTION public.fixture_image_complete_failure();").execute(pool).await.unwrap();
    } else {
        sqlx::raw_sql("DROP TRIGGER fixture_image_complete_failure ON core_image.operations; DROP FUNCTION public.fixture_image_complete_failure();").execute(pool).await.unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn image_recovery_limit_one_passes_earlier_unknown_across_rounds_without_redispatch(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::NativeImageByPrompt).await;
    let service = native_image_service(&f, &supplier);
    let mut unknown = input(1);
    unknown.model = "gpt-image-2".into();
    unknown.prompt = "unknown-cache".into();
    let early = format!("{:064x}", 1);
    let later = format!("{:064x}", 2);
    assert!(
        service
            .execute_api(&f.secret, &early, &groups(), unknown.clone(), None, None)
            .await
            .is_err()
    );
    let mut known = unknown;
    known.prompt = "known-cache".into();
    block_image_completion(&f.pool, true).await;
    assert!(
        service
            .execute_api(&f.secret, &later, &groups(), known.clone(), None, None)
            .await
            .is_err()
    );
    block_image_completion(&f.pool, false).await;
    assert_eq!(service.recover_receipts(1).await.unwrap(), 0);
    assert_eq!(service.recover_receipts(1).await.unwrap(), 1);
    let receipt = service
        .execute_api(&f.secret, &later, &groups(), known, None, None)
        .await
        .unwrap();
    assert_eq!(receipt.charged_credits, Some(381));
    assert_eq!(service.recover_receipts(1).await.unwrap(), 0);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 2);
    let pending = service
        .charging
        .resolve(&operation(&early))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.state, "reconcile");
    assert_eq!(pending.reserved, 1900);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((funds.available_units, funds.reserved_units), (7719, 1900));
}

#[sqlx::test(migrations = false)]
async fn image_api_legacy_ratio_uses_measured_totals_not_gpt_expression_or_per_image_price(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::NativeImageUnknownCache).await;
    let mut service = native_image_service(&f, &supplier);
    let mut prices = (*service.prices).clone();
    for rule in &mut prices.rules {
        rule.semantic_model = "google/gemini-3.1-flash-image".into();
        rule.count_basis = ImageCountBasis::LegacyTokenRatio(
            crate::billing::image_price::native_tokens::tests::ratio_tariff(),
        );
    }
    service.prices = Arc::new(prices);
    let mut request = input(1);
    request.model = "google/gemini-3.1-flash-image".into();
    let receipt = service
        .execute_api(
            &f.secret,
            &format!("{:064x}", 1104),
            &groups(),
            request,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(receipt.reservation_credits, 178);
    assert_eq!(receipt.charged_credits, Some(31));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((funds.available_units, funds.reserved_units), (9969, 0));
}

#[sqlx::test(migrations = false)]
async fn image_api_native_usage_frozen_expression_settlement_replay_and_receipt_recovery(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::NativeImage).await;
    let mut service = native_image_service(&f, &supplier);
    let mut request = input(1);
    request.model = "gpt-image-2".into();
    let key = format!("{:064x}", 1101);
    let receipt = service
        .execute_api(&f.secret, &key, &groups(), request.clone(), None, None)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 1900);
    assert_eq!(receipt.charged_credits, Some(381));
    assert_eq!(
        receipt.output.as_ref().unwrap()["usage"],
        crate::billing::image_price::native_tokens::tests::measured()
    );
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.price.cost(charge.usage).unwrap(), 381);
    let raw = serde_json::to_vec(&charge.usage.image).unwrap();
    let restored = serde_json::from_slice::<Option<ImageUsage>>(&raw).unwrap();
    assert_eq!(restored, charge.usage.image);
    let upload = ImageMultipartInput {
        images: vec![ImageUpload {
            bytes: base64::Engine::decode(&base64::engine::general_purpose::STANDARD,
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII=").unwrap(),
            media_type: ImageMediaType::Png,
        }],
        mask: None,
    };
    let edited = service
        .execute_api(
            &f.secret,
            &format!("{:064x}", 1103),
            &groups(),
            request.clone(),
            Some(upload),
            None,
        )
        .await
        .unwrap();
    assert_eq!(edited.charged_credits, Some(381));
    // Fail the real completion transaction after Charging.settle commits; never
    // violate the database's immutable completed-state fence to fake a crash.
    block_image_completion(&f.pool, true).await;
    let recovery_key = format!("{:064x}", 1105);
    assert!(
        service
            .execute_api(
                &f.secret,
                &recovery_key,
                &groups(),
                request.clone(),
                None,
                None
            )
            .await
            .is_err()
    );
    block_image_completion(&f.pool, false).await;
    let mut prices = (*service.prices).clone();
    prices.revision += 1;
    for rule in &mut prices.rules {
        if let ImageCountBasis::NativeTokens(tariff) = &mut rule.count_basis {
            tariff.source_expression = tariff.source_expression.replace("p * 5", "p * 6");
            tariff.source_sha256 =
                crate::hex_codec::encode(Sha256::digest(tariff.source_expression.as_bytes()));
        }
    }
    service.prices = Arc::new(prices);
    assert_eq!(service.recover_receipts(1).await.unwrap(), 1);
    let replay = service
        .execute_api(&f.secret, &key, &groups(), request, None, None)
        .await
        .unwrap();
    assert_eq!(replay.charged_credits, Some(381));
    assert_eq!(
        supplier.hits.load(Ordering::SeqCst),
        3,
        "never redispatch on recovery/replay"
    );
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((funds.available_units, funds.reserved_units), (8857, 0));
}

#[sqlx::test(migrations = false)]
async fn image_api_native_usage_unknown_cache_retains_hold_without_retry(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::NativeImageUnknownCache).await;
    let service = native_image_service(&f, &supplier);
    let mut request = input(1);
    request.model = "gpt-image-2".into();
    let key = format!("{:064x}", 1102);
    assert!(
        service
            .execute_api(&f.secret, &key, &groups(), request.clone(), None, None)
            .await
            .is_err()
    );
    let charge = service
        .charging
        .resolve(&operation(&key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.state, "reconcile");
    assert_eq!((charge.reserved, charge.settled), (1900, 0));
    assert_eq!(service.recover_receipts(1).await.unwrap(), 0);
    let _ = service
        .execute_api(&f.secret, &key, &groups(), request, None, None)
        .await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((funds.available_units, funds.reserved_units), (8100, 1900));
}

#[sqlx::test(migrations = false)]
async fn image_api_pg_request_limits_reject_before_image_hold_and_supplier(pool: PgPool) {
    let f = fixture(pool).await;
    sqlx::query("SELECT core_request_limits.write_policy(1,TRUE,86400,1,'{}',7)")
        .execute(&f.pool)
        .await
        .unwrap();
    crate::request_limits::admit(&f.pool, 7, "allowed", "existing-request")
        .await
        .unwrap();
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    let result = service
        .execute_api(&f.secret, &"d".repeat(64), &groups(), input(1), None, None)
        .await;
    assert!(matches!(result,Err(Error::RateLimited(seconds)) if (1..=86400).contains(&seconds)));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    no_charges(&f).await;
}

#[sqlx::test(migrations = false)]
async fn image_api_actual_payload_count_settlement_replay_and_conflict(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    let key = format!("{:064x}", 1);
    let receipt = service
        .execute_api(&f.secret, &key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(receipt.operation_id, operation(&key));
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 6000);
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        funds.available_units, 8000,
        "the unused two-image reservation must be returned"
    );
    assert_eq!(funds.reserved_units, 0);
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.key_id, Some(f.key_id));
    assert_eq!(charge.owner_account_id, f.account);
    assert_eq!(charge.payer_account_id, f.account);
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    assert_eq!(charge.usage.image.unwrap().output_images, 1);
    assert_eq!(
        (charge.usage.input, charge.usage.cached, charge.usage.output),
        (0, 0, 0)
    );
    let repeated = service
        .execute_api(&f.secret, &key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(repeated.charged_credits, Some(2000));
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let mut changed = input(3);
    changed.prompt.push_str(" changed");
    assert_eq!(
        service
            .execute_api(&f.secret, &key, &groups(), changed, None, None)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE key_id=$1")
            .bind(f.key_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        8000
    );
}

#[sqlx::test(migrations = false)]
async fn image_api_empty_or_truncated_provider_retains_full_hold_without_retry(pool: PgPool) {
    let f = fixture(pool).await;
    for (position, reply) in [Reply::Empty, Reply::Truncated].into_iter().enumerate() {
        let supplier = Supplier::start(reply).await;
        let service = image_service(&f, &supplier);
        let key = format!("{:064x}", position + 10);
        assert_eq!(
            service
                .execute_api(&f.secret, &key, &groups(), input(2), None, None)
                .await
                .unwrap_err(),
            Error::Pending
        );
        let charge = service
            .charging
            .resolve(&operation(&key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.state, "reconcile");
        assert_eq!(charge.reserved, 4000);
        assert_eq!(charge.settled, 0);
        assert!(charge.usage.image.is_none());
        let funds = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(funds.reserved_units, (position as i64 + 1) * 4000);
        assert_eq!(funds.available_units, 10000 - (position as i64 + 1) * 4000);
        let repeated = service
            .execute_api(&f.secret, &key, &groups(), input(2), None, None)
            .await
            .unwrap();
        assert_ne!(repeated.state, "completed");
        assert!(repeated.output.is_none());
        assert!(repeated.charged_credits.is_none());
        assert_eq!(
            supplier.hits.load(Ordering::SeqCst),
            1,
            "unknown provider outcome must never trigger another POST"
        );
        assert!(
            service
                .charging
                .recover_expired(10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service.recover_receipts(10).await.unwrap(),
            0,
            "missing payload is not settlement evidence"
        );
        assert_eq!(
            f.ledger.balance(f.account).await.unwrap().reserved_units,
            (position as i64 + 1) * 4000
        );
    }
}

#[sqlx::test(migrations = false)]
async fn image_api_real_key_model_group_peer_policies_reject_before_hold(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits,allow_ips) VALUES($1,'synthetic-image-restricted-key','allowed',TRUE,ARRAY['other-model'],ARRAY['127.0.0.1/32'::inet])")
        .bind(f.key_id).execute(&f.pool).await.unwrap();
    let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let forbidden = Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
    let first = format!("{:064x}", 20);
    assert_eq!(
        service
            .execute_api(&f.secret, &first, &groups(), input(1), None, loopback)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY['dall-e-3'],group_name='different-group' WHERE credential_id=$1")
        .bind(f.key_id).execute(&f.pool).await.unwrap();
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 21),
                &groups(),
                input(1),
                None,
                loopback
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    sqlx::query(
        "UPDATE core_identity.api_key_settings SET group_name='allowed' WHERE credential_id=$1",
    )
    .bind(f.key_id)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 22),
                &groups(),
                input(1),
                None,
                forbidden
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 23),
                &groups(),
                input(1),
                None,
                None
            )
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    no_charges(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    let completed = service
        .execute_api(
            &f.secret,
            &format!("{:064x}", 24),
            &groups(),
            input(1),
            None,
            loopback,
        )
        .await
        .unwrap();
    assert_eq!(completed.charged_credits, Some(2000));
    assert_eq!(
        service
            .query_api_with_peer(&f.secret, &completed.operation_id, forbidden)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(
        service
            .query_api_with_peer(&f.secret, &completed.operation_id, None)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrations = false)]
async fn image_api_malformed_edit_upload_rejects_before_hold_and_post(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service(&f, &supplier);
    let edits = ImageMultipartInput {
        images: vec![ImageUpload {
            bytes: b"not a PNG image".to_vec(),
            media_type: ImageMediaType::Png,
        }],
        mask: None,
    };
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 30),
                &groups(),
                input(1),
                Some(edits),
                None
            )
            .await
            .unwrap_err(),
        Error::Invalid
    );
    no_charges(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test(migrations = false)]
async fn image_api_disconnected_caller_drain_preserves_worker_and_settlement(pool: PgPool) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::SlowOneImage).await;
    let service = image_service(&f, &supplier);
    let request_key = format!("{:064x}", 40);
    let caller_service = service.clone();
    let caller_secret = f.secret.clone();
    let caller_key = request_key.clone();
    let caller = tokio::spawn(async move {
        caller_service
            .execute_api(&caller_secret, &caller_key, &groups(), input(3), None, None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let hits = supplier.hits.load(Ordering::SeqCst);
            assert!(hits <= 1);
            if hits == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the provider must observe the original POST before caller disconnect");
    let dispatched = service
        .charging
        .resolve(&operation(&request_key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dispatched.state, "streaming");
    assert_eq!(dispatched.reserved, 6000);
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 4000);
    assert_eq!(funds.reserved_units, 6000);
    // Abort the HTTP caller's awaiting future, never the service-owned worker.
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(service.drain(Duration::from_millis(10)).await, 1);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    supplier.release.add_permits(1);
    let receipt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = match service
                .query_api_with_peer(&f.secret, &operation(&request_key), None)
                .await
            {
                Ok(receipt) => receipt,
                Err(Error::Retry) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(error) => panic!("receipt query failed during detached completion: {error:?}"),
            };
            if receipt.state == "completed" {
                break receipt;
            }
            assert!(
                receipt.output.is_none(),
                "unsettled output must not be delivered"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect(
        "the detached worker must persist and settle the original image after caller disconnect",
    );
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let funds = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(funds.available_units, 8000);
    assert_eq!(funds.reserved_units, 0);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(service.drain(Duration::from_secs(1)).await, 0);
    assert_eq!(
        service
            .execute_api(
                &f.secret,
                &format!("{:064x}", 41),
                &groups(),
                input(1),
                None,
                None
            )
            .await
            .unwrap_err(),
        Error::Pending
    );
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let charges: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(charges, 1);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        8000
    );
}

pub(super) use crate::test_runtime as runtime_role;

#[sqlx::test(migrations = false)]
async fn image_api_restricted_runtime_role_generates_and_settles_real_funds(pool: PgPool) {
    let f = fixture(pool).await;
    let role = runtime_role::RuntimeRole::create(&f.pool).await;
    for capability in ["core_charging", "core_model", "core_image"] {
        role.optional(capability).await;
    }
    crate::runtime_database::check(
        &role.pool,
        crate::runtime_database::RequiredCapabilities {
            models: true,
            images: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&role.pool, &supplier);
    service.ready().await.unwrap();
    use tower::ServiceExt;
    let readiness =
        crate::model_http::image_internal::internal_router(service.clone(), vec![b'r'; 32])
            .unwrap();
    for (body, status) in [
        ("{}", axum::http::StatusCode::OK),
        ("{\"ready\":true}", axum::http::StatusCode::BAD_REQUEST),
    ] {
        let response = readiness
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/internal/models/images/v1/ready")
                    .header("authorization", format!("Bearer {}", "r".repeat(32)))
                    .header("x-lmm-protocol", "2")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status.is_success() {
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                json!({"protocol":2,"service":"images","ready":true})
            );
        }
    }
    let request_key = format!("{:064x}", 50);
    let receipt = service
        .execute_api(&f.secret, &request_key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(receipt.reservation_credits, 6000);
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        receipt.output.as_ref().unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let charge = service
        .charging
        .resolve(&receipt.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.key_id, Some(f.key_id));
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.reserved, 6000);
    assert_eq!(charge.settled, 2000);
    let ledger = Ledger::from_pool(role.pool.clone());
    let balance = ledger.balance(f.account).await.unwrap();
    assert_eq!(balance.available_units, 8000);
    assert_eq!(balance.reserved_units, 0);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    let repeated = service
        .execute_api(&f.secret, &request_key, &groups(), input(3), None, None)
        .await
        .unwrap();
    assert_eq!(repeated.output, receipt.output);
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    assert_eq!(service.drain(Duration::from_secs(1)).await, 0);
    assert_eq!(service.ready().await, Err(Error::Pending));
    drop(ledger);
    drop(service);
    role.cleanup().await;
}

mod huawei_edits_http {
    use super::*;
    use crate::model_http::{ChargePolicy, ModelGateway};
    use crate::routing::{PriceConfig, PriceSnapshot, Router as ModelRouter};
    use axum::http::{Request, StatusCode};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::io::Write as _;
    use tower::ServiceExt;

    const MODEL: &str = "qwen-image-edit-2509";
    const SINGLE: &str = "qwen_image_edit";
    const CANONICAL: &str = "https://api.modelarts-maas.com/v1";
    const KEY: &str = "synthetic-huawei-image-key";
    const BOUNDARY: &str = "synthetic-huawei-edit-boundary";

    // Two valid 512-by-512 one-bit grayscale PNGs, independently decodable.
    fn png(shade: u8) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
            let mut crc = flate2::Crc::new();
            crc.update(tag);
            crc.update(data);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(tag);
            out.extend_from_slice(data);
            out.extend_from_slice(&crc.sum().to_be_bytes());
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut row = [shade; 65];
        row[0] = 0;
        for _ in 0..512 {
            encoder.write_all(&row).unwrap();
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &[0, 0, 2, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0]);
        chunk(&mut out, b"IDAT", &encoder.finish().unwrap());
        chunk(&mut out, b"IEND", &[]);
        out
    }

    async fn edits(
        app: &Router,
        secret: &str,
        id: &str,
        prompt: &str,
        images: &[Vec<u8>],
        model: &str,
    ) -> (StatusCode, String, Value) {
        let mut body = Vec::new();
        for (name, value) in [
            ("model", model),
            ("prompt", prompt),
            ("size", "512x512"),
            ("n", "1"),
            ("response_format", "b64_json"),
        ] {
            write!(
                body,
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .unwrap();
        }
        for (index, bytes) in images.iter().enumerate() {
            write!(body, "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"image[]\"; filename=\"ref-{index}.png\"\r\nContent-Type: image/png\r\n\r\n").unwrap();
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        write!(body, "--{BOUNDARY}--\r\n").unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::post("/v1/images/edits")
                    .header("authorization", format!("Bearer {secret}"))
                    .header("idempotency-key", id)
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={BOUNDARY}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let operation = response.headers()["x-lmm-image-operation-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let raw = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, operation, serde_json::from_slice(&raw).unwrap())
    }

    #[sqlx::test(migrations = false)]
    async fn ordinary_huawei_edit_http_bills_one_returned_image_and_keeps_unknown_hold(
        pool: PgPool,
    ) {
        let f = fixture(pool).await;
        let images = [png(0), png(255)];
        let encoded = STANDARD.encode(&images[0]);
        let first = format!("data:image/png;base64,{encoded}");
        let joined = format!(
            "{first},data:image/png;base64,{}",
            STANDARD.encode(&images[1])
        );
        let supplied = json!({"model":MODEL,"created":1762134592684_i64,"data":[{"url":null,"b64_json":first}],
            "usage":{"model_latency":39800,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"width":512,"height":512},"error":null});
        let mut single = supplied.clone();
        single["model"] = json!(SINGLE);
        single["usage"] =
            json!({"model_latency":17509,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0});
        let single_raw = single.to_string();
        let raw = format!("  {supplied}\n");
        let expected_sha: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let supplier = Router::new().route(
            "/v1/images/generations",
            post(move |headers: HeaderMap, Json(input): Json<Value>| {
                let (seen, joined, first, raw, single_raw) = (
                    seen.clone(),
                    joined.clone(),
                    first.clone(),
                    raw.clone(),
                    single_raw.clone(),
                );
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        headers["authorization"].to_str().unwrap(),
                        format!("Bearer {KEY}")
                    );
                    let prompt = input["prompt"].as_str().unwrap();
                    let (model, reference, complete) = match input["model"].as_str().unwrap() {
                        MODEL => (MODEL, joined, raw),
                        SINGLE => (SINGLE, first, single_raw),
                        other => panic!("unexpected image model {other}"),
                    };
                    assert_eq!(
                        input,
                        json!({"model":model,"prompt":prompt,"image":reference,"size":"512x512"})
                    );
                    let payload = if prompt == "finish" {
                        complete
                    } else {
                        "{\"data\":[".to_owned()
                    };
                    ([("content-type", "application/json")], payload)
                }
            }),
        );
        let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", socket.local_addr().unwrap());
        let http = tokio::spawn(async move { axum::serve(socket, supplier).await.unwrap() });
        let limits = Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        };
        let rule = ImagePriceRule {
            upstream_id: 1,
            vendor: "huawei".into(),
            billing_group: "allowed".into(),
            upstream_model: MODEL.into(),
            semantic_model: MODEL.into(),
            endpoint: ImageEndpoint::Edits,
            size: "512x512".into(),
            quality: String::new(),
            max_n: 1,
            usd_micros_per_image: ExactRatio {
                numerator: 4000,
                denominator: 1,
            },
            group_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_budget: Credits::try_from(10000).unwrap(),
            count_basis: ImageCountBasis::ReturnedPayloads,
            source_revision: "synthetic-huawei-image-price".into(),
        };
        let mut single_rule = rule.clone();
        single_rule.semantic_model = SINGLE.into();
        single_rule.upstream_model = SINGLE.into();
        let spec = |model: &str| ProviderSpec {
            upstream_id: 1,
            vendor: "huawei".into(),
            upstream_model: model.into(),
            base_url: base.clone(),
            secret: KEY.into(),
            local_http: true,
        };
        let mut service = ImageService::new(
            f.pool.clone(),
            Charging::new(f.pool.clone(), Arc::new(TransactionalLedger)),
            ImagePriceConfig {
                revision: 1,
                rules: vec![rule, single_rule],
            },
            vec![spec(MODEL), spec(SINGLE)],
            &limits,
        )
        .unwrap();
        for provider in Arc::get_mut(&mut service.providers).unwrap().values_mut() {
            provider.use_huawei_edit_fixture(CANONICAL).unwrap();
        }
        let routing = ModelRouter::new(serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"upstreams":[],"routes":[],"aliases":[]})).unwrap(),
            Arc::new(PriceSnapshot::compile(PriceConfig {version:1,rules:vec![]}).unwrap())).unwrap();
        let app = ModelGateway::new(
            f.pool.clone(),
            Arc::new(routing),
            vec![],
            BTreeMap::from([(f.key_id, "allowed".into())]),
            limits,
            ChargePolicy {
                input_token_ceiling: 100,
                lease_seconds: 30,
            },
        )
        .unwrap()
        .with_image_service(service.clone())
        .router();

        let missing = edits(
            &app,
            &f.secret,
            "no-price",
            "finish",
            &images,
            "unpriced-image",
        )
        .await;
        assert_eq!(missing.0, StatusCode::FORBIDDEN);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        no_charges(&f).await;
        let ok = edits(&app, &f.secret, "success", "finish", &images, MODEL).await;
        assert_eq!(ok.0, StatusCode::OK, "{}", ok.2);
        assert_eq!(ok.2["data"][0]["b64_json"], encoded);
        let charge = service.charging.resolve(&ok.1).await.unwrap().unwrap();
        assert_eq!(charge.state, "settled");
        assert_eq!((charge.reserved, charge.settled), (2000, 2000));
        assert_eq!(
            (charge.usage.input, charge.usage.cached, charge.usage.output),
            (0, 0, 0)
        );
        let evidence = charge.usage.image.unwrap();
        assert_eq!(evidence.output_images, 1);
        assert_eq!(evidence.provider_receipt_sha256, expected_sha);
        assert!(evidence.native_tokens.is_none());
        assert_eq!(
            edits(&app, &f.secret, "success", "finish", &images, MODEL).await,
            ok
        );
        assert_eq!(
            edits(&app, &f.secret, "success", "changed", &images, MODEL)
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let pending = edits(&app, &f.secret, "pending", "incomplete", &images, MODEL).await;
        assert_eq!(pending.0, StatusCode::BAD_GATEWAY, "{}", pending.2);
        assert_eq!(pending.2["error"]["code"], "settlement_pending");
        assert!(pending.2.get("data").is_none());
        assert_ne!(pending.1, ok.1);
        assert_eq!(
            edits(&app, &f.secret, "pending", "incomplete", &images, MODEL).await,
            pending
        );
        assert_eq!(service.recover_receipts(10).await.unwrap(), 0);
        let held = service.charging.resolve(&pending.1).await.unwrap().unwrap();
        assert_eq!(held.state, "reconcile");
        assert_eq!((held.reserved, held.settled), (2000, 0));
        assert!(held.usage.image.is_none());
        let balance = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(
            (balance.available_units, balance.reserved_units),
            (6000, 2000)
        );
        let requests: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE key_id=$1")
                .bind(f.key_id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(requests, 2);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        let single = edits(&app, &f.secret, "single", "finish", &images[..1], SINGLE).await;
        assert_eq!(single.0, StatusCode::OK, "{}", single.2);
        assert_eq!(single.2["data"][0]["b64_json"], encoded);
        let charge = service.charging.resolve(&single.1).await.unwrap().unwrap();
        assert_eq!((charge.reserved, charge.settled), (2000, 2000));
        let usage = charge.usage.image.unwrap();
        assert_eq!(usage.output_images, 1);
        assert!(usage.native_tokens.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        http.abort();
    }
}
