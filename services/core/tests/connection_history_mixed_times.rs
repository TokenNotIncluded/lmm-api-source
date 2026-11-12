//! Exercise the production historical aggregation with mixed valid/invalid times.
use sqlx::{PgPool, Row};

#[sqlx::test(migrations = false)]
async fn valid_observations_survive_pre_registration_events(pool: PgPool) {
    let production = include_str!("../src/community_read/connections.rs");
    let historical = production
        .split("), historical AS MATERIALIZED (")
        .nth(1)
        .unwrap()
        .split("), native AS (")
        .next()
        .unwrap();
    // Supply the already authenticated/mapped snapshot boundary; execute the
    // exact production aggregation against PostgreSQL, including completeness.
    let query = format!(
        r#"WITH mapped AS (
      SELECT 1::bigint AS id, 37::bigint AS source_user_id,
        100::bigint AS registered_at, 200::bigint AS snapshot_observed_at,
        '{{"oauth_server_grants":[{{"user_id":37,"created_at_ms":90000}},{{"user_id":37,"created_at_ms":110000}}],
          "developer_access_requests":[{{"user_id":37,"created_at":90,"status":"approved","reviewed_at":95}},
          {{"user_id":37,"created_at":120,"status":"approved","reviewed_at":130}}]}}'::jsonb AS document
    ), tokens AS (
      SELECT 1::bigint AS id, managed AS oauth_managed, at AS created_time
      FROM (VALUES(false,90::bigint),(false,110),(true,90),(true,115)) t(managed,at)
    ), historical AS MATERIALIZED ({historical}) SELECT * FROM historical"#
    );
    let row = sqlx::query(&query).fetch_one(&pool).await.unwrap();
    for (column, expected) in [
        ("key_at", 110_i64),
        ("oauth_at", 110),
        ("application_at", 120),
        ("approval_at", 130),
    ] {
        assert_eq!(row.get::<i64, _>(column), expected, "{column}");
    }
    for column in [
        "key_complete",
        "oauth_complete",
        "application_complete",
        "approval_complete",
    ] {
        assert!(
            !row.get::<bool, _>(column),
            "invalid observations must keep {column} false"
        );
    }
}
