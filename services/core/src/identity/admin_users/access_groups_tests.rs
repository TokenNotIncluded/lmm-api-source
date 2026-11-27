//! Real PostgreSQL + HTTP checks for the native users page's list and access contract.
use super::*;
use crate::runtime_database::{self, RuntimeDatabaseError};

async fn http_json(
    f: &Fixture,
    method: &str,
    path: &str,
    secret: Option<&str>,
    input: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(secret) = secret {
        request = request.header("authorization", format!("Bearer {secret}"));
    }
    let body = if let Some(value) = input {
        request = request.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&value).unwrap())
    } else {
        Body::empty()
    };
    let response = router(f.store.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["vary"], "Authorization");
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    if let Some(secret) = secret {
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));
    }
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn change(before: &Value, groups: &[&str]) -> Value {
    json!({
        "expected_auth_version": before["auth_version"],
        "expected": {"groups": before["groups"]},
        "groups": groups,
        "reason": "Synthetic routing access change"
    })
}

#[sqlx::test(migrations = false)]
async fn admin_user_teams_http_contract_tracks_membership_and_account_state(pool: PgPool) {
    let f = fresh(&pool).await;
    let owner = person(&f, &pool, 10, 0, "team_owner").await;
    let member = person(&f, &pool, 11, 0, "team_member").await;
    person(&f, &pool, 12, 0, "without_team").await;
    let team = f.identity.create_team(&owner.secret).await.unwrap();
    f.identity
        .rename_team(&owner.secret, team, "Product Team", 1)
        .await
        .unwrap();
    let invite = f
        .identity
        .invite(
            &owner.secret,
            team,
            &InviteRequest {
                recipient_user_id: 11,
                role: TeamRole::Member,
                can_spend: true,
                ttl_seconds: 3600,
            },
        )
        .await
        .unwrap();
    f.identity
        .accept_invite(&member.secret, &invite.secret)
        .await
        .unwrap();

    let (status, all) = http_json(&f, "GET",
        "/core/v1/admin/users?page=1&page_size=20&keyword=&group=&role=&status=&sort_by=id&sort_order=desc",
        Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["version"], 1);
    assert_eq!(
        all["capabilities"]["filters"],
        json!(["keyword", "group", "role", "status", "team_id"])
    );
    assert!(
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|user| user["teams"].is_array())
    );
    assert_eq!(
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["id"] == "12")
            .unwrap()["teams"],
        json!([])
    );

    let uri = format!("/core/v1/admin/users?team_id={team}&sort_by=id&sort_order=asc");
    let (status, selected) = http_json(&f, "GET", &uri, Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(selected["total"], 2);
    let owner_team = &selected["items"][0]["teams"][0];
    let member_team = &selected["items"][1]["teams"][0];
    assert_eq!(owner_team["id"], team.to_string());
    assert!(
        owner_team["account_id"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap()
            > 0
    );
    assert_eq!(owner_team["name"], "Product Team");
    assert_eq!(owner_team["role"], "owner");
    assert_eq!(owner_team["can_spend"], true);
    assert_eq!(member_team["role"], "member");
    assert_eq!(member_team["can_spend"], true);

    let (status, detail) =
        http_json(&f, "GET", "/core/v1/admin/users/11", Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["user"]["teams"], selected["items"][1]["teams"]);
    for invalid in ["", "0", "-1", "not-an-id", "9223372036854775808"] {
        assert_eq!(
            http_json(
                &f,
                "GET",
                &format!("/core/v1/admin/users?team_id={invalid}"),
                Some(&f.admin),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }

    f.identity
        .remove_member(&owner.secret, team, 11, 1)
        .await
        .unwrap();
    let (_, selected) = http_json(&f, "GET", &uri, Some(&f.admin), None).await;
    assert_eq!(selected["total"], 1);
    let (_, detail) = http_json(&f, "GET", "/core/v1/admin/users/11", Some(&f.admin), None).await;
    assert_eq!(detail["user"]["teams"], json!([]));
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT account_id FROM core_identity.teams WHERE id=$1)")
        .bind(team).execute(&pool).await.unwrap();
    assert_eq!(
        http_json(&f, "GET", &uri, Some(&f.admin), None).await.1["total"],
        0
    );
    sqlx::query("UPDATE core_identity.accounts SET active=TRUE WHERE id=(SELECT account_id FROM core_identity.teams WHERE id=$1)")
        .bind(team).execute(&pool).await.unwrap();
    f.identity.close_team(&owner.secret, team, 2).await.unwrap();
    assert_eq!(
        http_json(&f, "GET", &uri, Some(&f.admin), None).await.1["total"],
        0
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn admin_user_access_http_saves_real_groups_and_enforces_browser_contract(pool: PgPool) {
    let f = fresh(&pool).await;
    let target = person(&f, &pool, 10, 0, "access_target").await;
    let ordinary = person(&f, &pool, 11, 0, "access_ordinary").await;
    sqlx::query("UPDATE core_identity.accounts SET service_level=4 WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=10)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES(10,TRUE,'synthetic-access-test') ON CONFLICT(user_id) DO UPDATE SET granted=TRUE")
        .execute(&pool).await.unwrap();
    let url = "/core/v1/admin/user-access/10";
    let (status, before) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        before,
        json!({
            "contract": "core.admin-user-access.v1", "user_id": "10",
            "auth_version": before["auth_version"], "active": true, "account_active": true, "groups": []
        })
    );
    assert_eq!(
        http_json(&f, "GET", url, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http_json(&f, "GET", url, Some(&ordinary.secret), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        http_json(
            &f,
            "GET",
            "/core/v1/admin/user-access/901",
            Some(&f.admin),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let (status, updated) = http_json(
        &f,
        "PATCH",
        url,
        Some(&f.admin),
        Some(change(&before, &["Alpha", "beta"])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["changed"], true);
    assert_eq!(updated["user"]["groups"], json!(["Alpha", "beta"]));
    assert!(
        updated["user"]["auth_version"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap()
            > before["auth_version"]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap()
    );
    assert!(matches!(
        f.identity.authorize(&target.secret).await,
        Err(IdentityError::Unauthorized)
    ));
    let (_, detail) = http_json(&f, "GET", "/core/v1/admin/users/10", Some(&f.admin), None).await;
    assert_eq!(detail["user"]["groups"], updated["user"]["groups"]);
    let (_, groups) = http_json(
        &f,
        "GET",
        "/core/v1/admin/users/groups",
        Some(&f.admin),
        None,
    )
    .await;
    assert_eq!(groups["items"], json!(["Alpha", "beta"]));

    assert_eq!(
        http_json(
            &f,
            "PATCH",
            url,
            Some(&f.admin),
            Some(change(&before, &["other"]))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, noop) = http_json(
        &f,
        "PATCH",
        url,
        Some(&f.admin),
        Some(change(&updated["user"], &["Alpha", "beta"])),
    )
    .await;
    assert_eq!(noop["changed"], false);
    assert_eq!(noop["user"], updated["user"]);
    for invalid in [
        vec!["auto"],
        vec!["beta", "Alpha"],
        vec!["Alpha", "Alpha"],
        vec!["bad\nname"],
    ] {
        assert_eq!(
            http_json(
                &f,
                "PATCH",
                url,
                Some(&f.admin),
                Some(change(&updated["user"], &invalid))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut unknown_field = change(&updated["user"], &["Alpha"]);
    unknown_field["service_level"] = json!(0);
    assert_eq!(
        http_json(&f, "PATCH", url, Some(&f.admin), Some(unknown_field))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        http_json(
            &f,
            "PATCH",
            url,
            Some(&ordinary.secret),
            Some(change(&updated["user"], &["Alpha"]))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        http_json(
            &f,
            "PATCH",
            "/core/v1/admin/user-access/900",
            Some(&f.admin),
            Some(change(&updated["user"], &["Alpha"]))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut stale_groups = change(&updated["user"], &["replacement"]);
    stale_groups["expected"]["groups"] = json!([]);
    assert_eq!(
        http_json(&f, "PATCH", url, Some(&f.admin), Some(stale_groups))
            .await
            .0,
        StatusCode::CONFLICT
    );

    let (status, cleared) = http_json(
        &f,
        "PATCH",
        url,
        Some(&f.admin),
        Some(change(&updated["user"], &[])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared["changed"], true);
    assert_eq!(cleared["user"]["groups"], json!([]));
    let enabled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.user_routing_groups WHERE user_id=10 AND enabled",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(enabled, 0);
    let hidden: (i16, bool) = sqlx::query_as("SELECT a.service_level,d.granted FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.developer_access d ON d.user_id=u.id WHERE u.id=10")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(hidden, (4, true));
    let audit: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE actor_user_id=900 AND action='admin.user.routing_groups.update' AND target->>'user_id'='10'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        audit, 2,
        "rejected/no-op changes must not create successful mutation receipts"
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn admin_user_access_http_conflicts_and_audit_failure_are_atomic(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "atomic_target").await;
    let url = "/core/v1/admin/user-access/10";
    let (_, before) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    let (a, b) = tokio::join!(
        http_json(
            &f,
            "PATCH",
            url,
            Some(&f.admin),
            Some(change(&before, &["Alpha"]))
        ),
        http_json(
            &f,
            "PATCH",
            url,
            Some(&f.admin),
            Some(change(&before, &["beta"]))
        )
    );
    assert_eq!(
        usize::from(a.0 == StatusCode::OK) + usize::from(b.0 == StatusCode::OK),
        1
    );
    assert!(a.0 == StatusCode::CONFLICT || b.0 == StatusCode::CONFLICT);
    let (_, current) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    sqlx::raw_sql("CREATE FUNCTION core_identity.test_reject_access_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic access audit failure'; END $$; CREATE TRIGGER test_access_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION core_identity.test_reject_access_audit()")
        .execute(&pool).await.unwrap();
    assert_eq!(
        http_json(
            &f,
            "PATCH",
            url,
            Some(&f.admin),
            Some(change(&current, &["replacement"]))
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        http_json(&f, "GET", url, Some(&f.admin), None).await.1,
        current
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn group_options_show_published_names_and_actual_inheritance(pool: PgPool) {
    let f = fresh(&pool).await;
    let user = person(&f, &pool, 10, 0, "group_options").await;
    sqlx::query(r#"SELECT public.routing_stage_prices('{"version":1,"rules":[]}')"#)
        .execute(&pool)
        .await
        .unwrap();
    let config = json!({
        "version": 1, "upstreams": [], "routes": [], "aliases": [],
        "groups": [
            {"name":"default","enabled":true},
            {"name":"unassigned","enabled":true},
            {"name":"disabled","enabled":false}
        ]
    });
    sqlx::query("SELECT public.routing_publish_config($1,1,NULL,NULL)")
        .bind(config)
        .execute(&pool)
        .await
        .unwrap();
    let url = "/core/v1/admin/user-access/10/group-options";
    assert_eq!(
        http_json(&f, "GET", url, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http_json(&f, "GET", url, Some(&user.secret), None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, options) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(options["available"], json!(["default", "unassigned"]));
    assert_eq!(options["access"]["groups"], json!([]));
    assert_eq!(options["effective"], json!(["default"]));
    assert_eq!(options["inherited"], json!(["default"]));
    let edit = "/core/v1/admin/user-access/10";
    let (status, saved) = http_json(
        &f,
        "PATCH",
        edit,
        Some(&f.admin),
        Some(change(&options["access"], &["unassigned"])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, options) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    assert_eq!(options["effective"], json!(["unassigned"]));
    assert_eq!(options["inherited"], json!([]));
    let (status, _) = http_json(
        &f,
        "PATCH",
        edit,
        Some(&f.admin),
        Some(change(&saved["user"], &[])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, options) = http_json(&f, "GET", url, Some(&f.admin), None).await;
    assert_eq!(options["effective"], json!([]));
    assert_eq!(options["inherited"], json!([]));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn admin_user_access_runtime_requires_only_membership_columns(pool: PgPool) {
    let f = fresh(&pool).await;
    runtime_database::check(&f.role.pool, Default::default())
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE(enabled) ON core_identity.user_routing_groups FROM {}",
        f.role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&f.role.pool, Default::default()).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT UPDATE(enabled) ON core_identity.user_routing_groups TO {}",
        f.role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    for privilege in [
        "UPDATE(source_reference)",
        "UPDATE(user_id)",
        "DELETE",
        "INSERT",
        "UPDATE",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT {privilege} ON core_identity.user_routing_groups TO {}",
            f.role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&f.role.pool, Default::default()).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "accepted {privilege}"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE {privilege} ON core_identity.user_routing_groups FROM {}",
            f.role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        // Restoring the approved grants also covers PostgreSQL's table-level revoke semantics.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT INSERT(user_id,group_name,enabled,source_reference),UPDATE(enabled) ON core_identity.user_routing_groups TO {}", f.role.name
        ))).execute(&pool).await.unwrap();
        runtime_database::check(&f.role.pool, Default::default())
            .await
            .unwrap();
    }
    f.role.cleanup().await;
}
