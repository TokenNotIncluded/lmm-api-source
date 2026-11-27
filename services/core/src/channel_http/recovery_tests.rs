//! Real HTTP proof for recovered vault keys and copied channel bindings.
//! The existing origin mapping is cfg(test) only; persistent routes stay HTTPS.
use axum::{
    Router,
    http::{StatusCode, header},
};
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    routing::CredentialRef,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

use crate::test_runtime as runtime;

#[path = "../../tests/common/channel_control.rs"]
mod support;
use support::*;

#[sqlx::test(migrations = false)]
async fn vault_keys_persist_encrypt_rotate_retire_and_recover_into_real_transport(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_, _, admin, ordinary) = vault_app(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA public TO {0}; GRANT SELECT ON public.routing_active_release,public.routing_groups,public.routing_routes TO {0}", role.name))).execute(&pool).await.unwrap();
    lmm_core::runtime_database::check(
        &role.pool,
        lmm_core::runtime_database::RequiredCapabilities {
            models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for query in [
        "SELECT ciphertext FROM public.channel_secret_revisions",
        "SELECT document FROM public.channel_release_bindings",
    ] {
        let denied = sqlx::query(query).fetch_all(&role.pool).await.unwrap_err();
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("42501")
        );
    }
    let live_gateway = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let management = ManagedChannels::recover(role.pool.clone(), live_gateway.clone())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), management);
    let expected = Version::of(&fixture());
    let input = channel_input(expected, 2, "synthetic-vault-primary-key");
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels",
            &ordinary,
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut missing = input.clone();
    missing["contexts"] = json!([]);
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels",
            &admin,
            missing
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let empty: i64 = sqlx::query_scalar("SELECT count(*) FROM public.channel_secret_revisions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        empty, 0,
        "a failed binding must roll back its private revision"
    );
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        input,
    )
    .await;
    let mut document = active(&pool).await;
    let primary = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == 2)
        .unwrap()
        .credential;
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM public.channel_secret_revisions WHERE id=$1 AND revision=$2",
    )
    .bind(primary.id as i64)
    .bind(primary.revision as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !ciphertext
            .windows(27)
            .any(|window| window == b"synthetic-vault-primary-key")
    );
    let (_, list) = request(
        app.clone(),
        "GET",
        "/api/channel/search?keyword=Published",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["name"], "Published provider");
    assert!(!list.to_string().contains("second.example"));
    success(
        &app,
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":Version::of(&document),"enabled":false}),
    )
    .await;
    document = active(&pool).await;
    let added=success(&app,"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"add","expected_version":Version::of(&document),"label":"Rotation","secret":"synthetic-vault-secondary-key","activate":false})).await;
    let secondary: CredentialRef = serde_json::from_value(added["reference"].clone()).unwrap();
    document = active(&pool).await;
    assert_eq!(
        document
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == 2)
            .unwrap()
            .credential,
        primary
    );
    let listed = success(
        &app,
        "GET",
        "/core/v1/admin/routing/channels/2/keys",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(listed["keys"].as_array().unwrap().len(), 2);
    assert!(!listed.to_string().contains("synthetic-vault"));
    success(&app,"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"activate","expected_version":Version::of(&document),"reference":secondary})).await;
    document = active(&pool).await;
    assert_eq!(
        document
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == 2)
            .unwrap()
            .credential,
        secondary
    );
    assert_eq!(request(app.clone(),"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"remove","expected_version":Version::of(&document),"reference":secondary})).await.0,StatusCode::CONFLICT);
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels/2/keys",
        &admin,
        json!({"operation":"remove","expected_version":Version::of(&document),"reference":primary}),
    )
    .await;
    document = active(&pool).await;
    assert_eq!(request(app.clone(),"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"activate","expected_version":Version::of(&document),"reference":primary})).await.0,StatusCode::NOT_FOUND);
    let raw_audit: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(target::text,''),'') FROM core_identity.audit",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!raw_audit.contains("synthetic-vault"));
    assert!(
        ManagedChannels::recover(
            role.pool.clone(),
            gateway(&role.pool)
                .with_channel_secret_key(&[84; 32])
                .unwrap()
        )
        .await
        .is_err()
    );
    drop(app);
    // Only the cfg(test) transport maps this persisted HTTPS origin to the
    // numeric loopback server. Recovery and the handler keep the durable route.
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let provider = axum::Router::new().route(
        "/v1/models",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = seen.clone();
            async move {
                assert_eq!(
                    headers.get(header::AUTHORIZATION).unwrap(),
                    "Bearer synthetic-vault-secondary-key"
                );
                seen.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[{"id":"model-a"}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let restored_gateway = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap()
        .with_local_streaming()
        .with_balance_probe_loopback("https://second.example", &format!("http://{address}/"))
        .unwrap();
    let restored = ManagedChannels::recover(role.pool.clone(), restored_gateway.clone())
        .await
        .unwrap();
    assert_eq!(
        restored_gateway
            .oauth_catalog_surface()
            .project(&["default".into()], "https://resource.example", 1)
            .unwrap()
            .models
            .len(),
        1
    );
    let restored_app = managed_router(role.pool.clone(), restored);
    let probe = success(
        &restored_app,
        "GET",
        "/api/channel/test/2",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(probe["models"], json!(["model-a"]));
    assert_eq!(probe["inference_tested"], false);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    document = active(&pool).await;
    let synchronized = success(
        &restored_app,
        "POST",
        "/core/v1/admin/routing/channels/2/models/synchronize",
        &admin,
        json!({"expected_version": Version::of(&document)}),
    )
    .await;
    assert_eq!(synchronized["models"], json!(["model-a"]));
    let configuration = success(
        &restored_app,
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(
        configuration["bindings"]["discoveries"]["2"]["models"],
        json!(["model-a"])
    );
    document = active(&pool).await;
    success(
        &restored_app,
        "POST",
        "/core/v1/admin/routing/channels/2/keys",
        &admin,
        json!({"operation":"add","expected_version":Version::of(&document),
            "label":"Different catalog","secret":"synthetic-vault-replacement-key","activate":true}),
    )
    .await;
    let refreshed = success(
        &restored_app,
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin,
        Value::Null,
    )
    .await;
    assert!(refreshed["bindings"]["discoveries"].get("2").is_none());
    server.abort();
    drop(restored_app);
    drop(restored_gateway);
    drop(live_gateway);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn channel_copy_publishes_independent_keys_and_executable_bindings(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_, _, admin, ordinary) = vault_app(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    let live = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), live.clone())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let expected = Version::from(live.dashboard_routing().version().unwrap());
    let input = json!({"expected_version":expected,"suffix":"_copy"});
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &ordinary,
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (first, second) = tokio::join!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            input.clone()
        ),
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            input
        ),
    );
    let (status, reply) = if first.0 == StatusCode::OK {
        assert_eq!(second.0, StatusCode::CONFLICT, "{second:?}");
        first
    } else {
        assert_eq!(first.0, StatusCode::CONFLICT, "{first:?}");
        second
    };
    assert_eq!(status, StatusCode::OK, "{reply}");
    let id = reply["data"]["id"].as_u64().unwrap();
    assert_ne!(id, 1);
    assert_eq!(
        reply["data"]["version"],
        json!({"routing":expected.routing+1,"pricing":expected.pricing})
    );
    assert!(!reply.to_string().contains("synthetic-channel-only"));
    let document = active(&pool).await;
    let source = &document.routing.upstreams[0];
    let copied = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == id)
        .unwrap();
    assert_eq!(source.id, 1);
    assert_eq!(copied.endpoint, source.endpoint);
    assert_eq!(copied.protocol, source.protocol);
    assert_eq!(copied.enabled, source.enabled);
    assert_ne!(copied.credential, source.credential);
    assert_eq!(document.pricing, fixture().pricing);
    assert_eq!(document.routing.routes[0].targets.len(), 2);
    let original_target = &document.routing.routes[0].targets[0];
    let copied_target = &document.routing.routes[0].targets[1];
    assert_eq!(copied_target.upstream_id, id);
    assert_ne!(copied_target.id, original_target.id);
    assert_eq!(copied_target.upstream_model, original_target.upstream_model);
    assert_eq!(copied_target.weight, original_target.weight);
    assert_eq!(copied_target.priority, original_target.priority);
    let detail = success(
        &app,
        "GET",
        &format!("/api/channel/{id}"),
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(detail["name"], "Channel 1_copy");
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.channel_secret_revisions WHERE upstream_id=$1",
    )
    .bind(id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(keys, 1, "copy must own its independent encrypted key");
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM public.channel_secret_revisions WHERE upstream_id=$1",
    )
    .bind(id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !ciphertext
            .windows(b"synthetic-channel-only".len())
            .any(|s| s == b"synthetic-channel-only")
    );
    let denied = sqlx::query("SELECT ciphertext FROM public.channel_secret_revisions")
        .fetch_all(&role.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
    let audit: Value =
        sqlx::query_scalar("SELECT target FROM core_identity.audit WHERE action='channel.copy'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        audit["change"],
        json!({"source_id":1,"id":id,"reset_balance":true})
    );
    success(&app, "POST", "/core/v1/admin/routing/channels/1/keys", &admin,
        json!({"operation":"add","expected_version":Version::of(&document),"label":"Source rotation","secret":"synthetic-source-rotated","activate":true})).await;
    let after_rotation = active(&pool).await;
    assert_eq!(
        after_rotation
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == id)
            .unwrap()
            .credential,
        copied.credential
    );
    success(
        &app,
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":Version::of(&after_rotation),"enabled":false}),
    )
    .await;
    drop(app);
    drop(live);
    // The copied channel keeps its persisted HTTPS contract through recovery;
    // only this crate-local test maps provider transport to numeric loopback.
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let provider = Router::new().route(
        "/v1/models",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = seen.clone();
            async move {
                assert_eq!(
                    headers.get(header::AUTHORIZATION).unwrap(),
                    "Bearer synthetic-channel-only"
                );
                seen.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[{"id":"model-a"}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let restored = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap()
        .with_local_streaming()
        .with_balance_probe_loopback("https://provider.example", &format!("http://{address}/"))
        .unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), restored.clone())
        .await
        .unwrap();
    assert!(
        restored.dashboard_routing().route(route_request()).is_ok(),
        "copy must remain routable after disabling its source"
    );
    let app = managed_router(role.pool.clone(), manager);
    let probe = success(
        &app,
        "GET",
        &format!("/api/channel/test/{id}"),
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(probe["models"], json!(["model-a"]));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    server.abort();
    drop(app);
    drop(restored);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn assistant_model_settings_publish_recover_and_preserve_other_routing_configuration(
    pool: PgPool,
) {
    use lmm_core::assistant_model::PersonalTextPolicy;
    use lmm_core::channel_http::assistant_model_settings;
    let (channels, gateway, admin, ordinary) = vault_app(&pool).await;
    let policy: PersonalTextPolicy = serde_json::from_value(json!({
        "model":"model-a","group":"default","budget_credits":1000,"max_output_tokens":16,"ttl_seconds":60
    })).unwrap();
    let settings_path = "/api/option/assistant-model";
    let configuration_path = "/core/v1/admin/routing/configuration";
    let app = channels.merge(assistant_model_settings(gateway.clone(), &policy));
    assert_eq!(
        request(app.clone(), "GET", settings_path, &ordinary, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let initial = success(&app, "GET", settings_path, &admin, Value::Null).await;
    assert_eq!(initial["model"], "model-a");
    assert_eq!(initial["models"], json!(["model-a"]));

    let cfg = success(&app, "GET", configuration_path, &admin, Value::Null).await;
    let mut routing = cfg["routing"].clone();
    routing["version"] = json!(2);
    let mut route = routing["routes"][0].clone();
    route["id"] = json!(2);
    route["model"] = json!("model-b");
    route["targets"][0]["id"] = json!(2);
    route["targets"][0]["upstream_model"] = json!("model-b");
    routing["routes"].as_array_mut().unwrap().push(route);
    routing["groups"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"secondary","enabled":true}));
    let mut secondary = routing["routes"][0].clone();
    secondary["id"] = json!(3);
    secondary["group"] = json!("secondary");
    secondary["targets"][0]["id"] = json!(3);
    routing["routes"].as_array_mut().unwrap().push(secondary);
    routing["aliases"] =
        json!([{"group":"default","model":"model-b","alias":"assistant-alternate"}]);

    let mut prices = cfg["prices"].clone();
    prices["version"] = json!(2);
    let mut rule = prices["rules"][0].clone();
    rule["id"] = json!(2);
    rule["model"] = json!("model-b");
    rule["rates"]["input"] = json!(100_000);
    rule["rates"]["output"] = json!(200_000);
    prices["rules"].as_array_mut().unwrap().push(rule);

    let mut secondary_price = prices["rules"][0].clone();
    secondary_price["id"] = json!(3);
    secondary_price["group"] = json!("secondary");
    prices["rules"]
        .as_array_mut()
        .unwrap()
        .push(secondary_price);
    let mut bindings = cfg["bindings"].clone();
    let mut context = bindings["contexts"][0].clone();
    context["upstream_model"] = json!("model-b");
    bindings["contexts"].as_array_mut().unwrap().push(context);
    let mut change = json!({"expected_version":cfg["version"],"routing":routing,"prices":prices,"bindings":bindings});
    let preview_path = format!("{configuration_path}/preview");
    let preview = success(&app, "POST", &preview_path, &admin, change.clone()).await;
    change["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(&app, "PUT", configuration_path, &admin, change).await;
    let loaded = success(&app, "GET", settings_path, &admin, Value::Null).await;
    assert_eq!(
        loaded["models"],
        json!(["assistant-alternate", "model-a", "model-b"])
    );
    let selection = json!({"expected_version":loaded["version"], "model":"assistant-alternate"});
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            settings_path,
            &ordinary,
            selection.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            settings_path,
            &admin,
            json!({"expected_version":loaded["version"],"model":"missing"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let saved = success(&app, "PUT", settings_path, &admin, selection.clone()).await;
    assert_eq!(saved["model"], "assistant-alternate");
    assert_eq!(
        gateway.assistant_model_override().await.unwrap().as_deref(),
        Some("assistant-alternate")
    );
    crate::assistant_model::check_selected_model_issuance(
        &pool,
        gateway.clone(),
        &ordinary,
        &admin,
        &policy,
    )
    .await;
    assert_eq!(
        request(app.clone(), "PUT", settings_path, &admin, selection)
            .await
            .0,
        StatusCode::CONFLICT
    );

    // Simulate a previous console version that serializes only its known bindings.
    let cfg = success(&app, "GET", configuration_path, &admin, Value::Null).await;
    let mut bindings = cfg["bindings"].clone();
    bindings.as_object_mut().unwrap().remove("assistant_model");
    let mut routing = cfg["routing"].clone();
    routing["version"] = json!(cfg["version"]["routing"].as_u64().unwrap() + 1);
    let mut change = json!({"expected_version":cfg["version"],"routing":routing,"prices":cfg["prices"],"bindings":bindings});
    let preview = success(&app, "POST", &preview_path, &admin, change.clone()).await;
    change["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(&app, "PUT", configuration_path, &admin, change).await;

    let fresh_gateway = support::gateway(&pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let recovered = ManagedChannels::recover(pool.clone(), fresh_gateway.clone())
        .await
        .unwrap();
    let restarted = managed_router(pool.clone(), recovered)
        .merge(assistant_model_settings(fresh_gateway, &policy));
    let restored = success(&restarted, "GET", settings_path, &admin, Value::Null).await;
    assert_eq!(restored["model"], "assistant-alternate");
    assert_eq!(restored["routes"]["secondary"], json!(["model-a"]));
    let moved = success(
        &restarted,
        "PUT",
        settings_path,
        &admin,
        json!({"model":"model-a","group":"secondary","expected_version":restored["version"]}),
    )
    .await;
    assert_eq!(moved["group"], "secondary");
    assert_eq!(request(restarted.clone(), "PUT",settings_path,&admin,
        json!({"model":"assistant-alternate","group":"secondary","expected_version":moved["version"]})).await.0,StatusCode::BAD_REQUEST);
    let reset = success(
        &restarted,
        "PUT",
        settings_path,
        &admin,
        json!({"model":"model-a","group":"default","expected_version":moved["version"]}),
    )
    .await;
    assert_eq!(reset["model"], reset["default_model"]);
    crate::assistant_model::check_replaced_model_grant(&pool, gateway, &ordinary, &policy).await;
}
