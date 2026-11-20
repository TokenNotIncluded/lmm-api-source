//! Test-only observation after an unexpected offline upgrade rejection.
//! Never samples before admission, retries a call, or changes its result.
use sqlx::{ConnectOptions, Connection, PgConnection, postgres::PgConnectOptions};
use std::io::Write;

async fn maintenance_snapshot(
    database: &str,
    database_oid: u32,
) -> Result<serde_json::Value, &'static str> {
    let maintenance_url =
        std::env::var("DATABASE_URL").map_err(|_| "maintenance_url_unavailable")?;
    let options: PgConnectOptions = maintenance_url
        .parse()
        .map_err(|_| "maintenance_options_invalid")?;
    let options = options.disable_statement_logging();
    // Reject an absent/default or target database before opening any connection.
    match options.get_database() {
        Some(observed) if !observed.is_empty() && observed != database => {}
        _ => return Err("maintenance_database_not_separate"),
    }
    let mut connection = PgConnection::connect_with(&options)
        .await
        .map_err(|_| "maintenance_connect_failed")?;
    let snapshot = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(
        r#"SELECT jsonb_build_object(
            'backends', COALESCE((
                SELECT jsonb_agg(jsonb_build_object(
                    'pid', pid, 'backend_type', backend_type, 'state', state
                ) ORDER BY pid)
                FROM pg_stat_activity WHERE datid=$1::bigint::oid
            ), '[]'::jsonb),
            'prepared_count', (
                SELECT count(*) FROM pg_prepared_xacts p
                JOIN pg_database d ON d.datname=p.database
                WHERE d.oid=$1::bigint::oid
            )
        )"#,
    )
    .bind(i64::from(database_oid))
    .fetch_one(&mut connection)
    .await;
    let closed = connection.close().await;
    let snapshot = snapshot.map_err(|_| "maintenance_observation_failed")?;
    closed.map_err(|_| "maintenance_close_failed")?;
    Ok(snapshot.0)
}

pub(super) async fn diagnose_unexpected_refusal(
    (ordinal, phase): (u8, &'static str),
    database: &str,
    database_oid: u32,
) {
    // The original refusal has already occurred. Never sample before the call,
    // retry it, or turn a diagnostic failure into a different gate failure.
    let observation = match tokio::time::timeout(
        std::time::Duration::from_secs(1),
        maintenance_snapshot(database, database_oid),
    )
    .await
    {
        Ok(Ok(snapshot)) => serde_json::json!({"status": "observed", "snapshot": snapshot}),
        Ok(Err(status)) => serde_json::json!({"status": status}),
        Err(_) => serde_json::json!({"status": "observation_timeout"}),
    };
    let _ = writeln!(
        std::io::stderr().lock(),
        "upgrade_refusal_diagnostic {}",
        serde_json::json!({"ordinal": ordinal, "phase": phase, "observation": observation})
    );
}
