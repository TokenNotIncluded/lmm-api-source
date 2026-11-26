//! Frozen lineage, exact request replay and least-privilege runtime grants.
use super::*;
use crate::core_schema::upgrade::payment_discounts::tests::engine_offline_boundary;
use crate::core_schema::{UpgradeRoles, upgrade_database};

#[test]
fn route_runtime89_exact_lineage_matches_fresh_install() {
    authorize_target().unwrap();
    let prior: serde_json::Value =
        serde_json::from_str(include_str!("assistant_weekly_discounts_target.json")).unwrap();
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    let fresh: serde_json::Value =
        serde_json::from_str(include_str!("fresh70_target.json")).unwrap();
    assert_eq!(ENGINE.predecessor_manifest, prior["manifest_sha256"]);
    assert_eq!(ENGINE.predecessor_catalog, prior["catalog_sha256"]);
    assert_eq!(ENGINE.target_manifest().unwrap(), target["manifest_sha256"]);
    assert_eq!(fresh["manifest_sha256"], target["manifest_sha256"]);
    assert_eq!(fresh["catalog_sha256"], target["catalog_sha256"]);
    assert_eq!(fresh["component_count"], 89);
}

pub(in crate::core_schema::upgrade) async fn upgrade_from_core88(
    url: &str,
    previous: &UpgradeRequest,
) {
    let request = UpgradeRequest {
        procedure: PROCEDURE.into(),
        upgrade_id: format!("{}-route89", previous.upgrade_id),
        predecessor_manifest_sha256: ENGINE.predecessor_manifest.into(),
        predecessor_catalog_sha256: ENGINE.predecessor_catalog.into(),
        successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
        ..previous.clone()
    };
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    for status in ["committed", "already_committed"] {
        engine_offline_boundary(&request).await;
        let result = upgrade_database(url, request.clone()).await.unwrap();
        assert_eq!(result.status, status);
        assert_eq!(result.components, 89);
        assert_eq!(result.successor_catalog_sha256, target["catalog_sha256"]);
    }
}

pub(in crate::core_schema::upgrade) async fn assert_runtime_acl(
    c: &mut PgConnection,
    roles: &UpgradeRoles,
    enabled: bool,
) {
    let state = sqlx::query(
        "SELECT
        (SELECT count(*) FROM core_meta.route_runtime_upgrades) AS receipts,
        (SELECT count(*) FROM core_channels.affinity_cache) AS mappings,
        (SELECT count(*) FROM core_channels.route_health) AS health_rows,
        has_table_privilege($1,'core_channels.affinity_cache','SELECT') AND
            has_table_privilege($1,'core_channels.affinity_cache','INSERT') AND
            has_table_privilege($1,'core_channels.affinity_cache','DELETE') AND
            has_column_privilege($1,'core_channels.affinity_cache','expires_at','UPDATE') AS cache_writer,
        has_table_privilege($1,'core_channels.route_health','SELECT') AND
            has_table_privilege($1,'core_channels.route_health','INSERT') AND
            has_column_privilege($1,'core_channels.route_health','generation','UPDATE') AS probe_writer,
        NOT has_table_privilege($1,'core_channels.affinity_cache','TRUNCATE,TRIGGER,REFERENCES') AND
            NOT has_column_privilege($1,'core_channels.affinity_cache','key_sha256','UPDATE') AND
            NOT has_table_privilege($1,'core_channels.route_health','DELETE,TRUNCATE,TRIGGER,REFERENCES') AND
            NOT has_column_privilege($1,'core_channels.route_health','credential_revision','UPDATE') AND
            NOT has_any_column_privilege($2,'core_channels.affinity_cache','SELECT,INSERT,UPDATE,REFERENCES') AND
            NOT has_any_column_privilege($2,'core_channels.route_health','SELECT,INSERT,UPDATE,REFERENCES') AND
            NOT has_table_privilege($1,'core_meta.route_runtime_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') AND
            NOT has_any_column_privilege($2,'core_meta.route_runtime_upgrades','SELECT,INSERT,UPDATE,REFERENCES') AS bounded",
    ).bind(&roles.runtime).bind(&roles.extensions).fetch_one(c).await.unwrap();
    assert_eq!(state.get::<i64, _>("receipts"), 1);
    assert_eq!(state.get::<i64, _>("mappings"), 0);
    assert_eq!(state.get::<i64, _>("health_rows"), 0);
    assert_eq!(state.get::<bool, _>("cache_writer"), enabled);
    assert_eq!(state.get::<bool, _>("probe_writer"), enabled);
    assert!(state.get::<bool, _>("bounded"));
}
