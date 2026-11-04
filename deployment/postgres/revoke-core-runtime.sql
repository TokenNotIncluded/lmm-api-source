-- Explicit offline revocation of optional lmm-core runtime grants.
--
-- This is a reviewed ACL reset, never an automatic startup mutation. It removes
-- only grants covering optional capability schemas (plus the optional assistant
-- tables in core_gift and the Web routing projections in public). Baseline
-- identity, ledger, history, abuse, gift qualification and billing grants stay
-- intact. Re-run grant-core-runtime.sql with the final feature flags afterwards.
--
-- The complete fresh core schema installs these schemas before this script is
-- run. No role passwords, ownership changes, wildcard grants or default ACLs.
-- @mark deployment.database-authority role=permissions
\set ON_ERROR_STOP on
\if :{?runtime_role}
\else
\set runtime_role lmm_core_runtime
\endif
\if :{?confirm_revoke}
\else
\set confirm_revoke false
\endif

\if :confirm_revoke
\else
\echo 'Refusing optional runtime ACL reset; pass -v confirm_revoke=true explicitly.'
DO $$ BEGIN
    RAISE EXCEPTION 'Refusing optional runtime ACL reset; pass -v confirm_revoke=true explicitly.';
END $$;
\endif

BEGIN;
SET LOCAL statement_timeout = '5s';
SELECT EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname = :'runtime_role' AND r.rolcanlogin
      AND NOT (r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls)
) AND EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname = 'lmm_core_owner' AND NOT r.rolcanlogin
) AND NOT EXISTS (
    SELECT FROM pg_roles r WHERE r.rolname <> :'runtime_role' AND (
      pg_has_role(:'runtime_role', r.oid, 'SET')
      OR pg_has_role(:'runtime_role', r.oid, 'MEMBER WITH ADMIN OPTION')
      OR pg_has_role(:'runtime_role', r.oid, 'USAGE'))
) AS runtime_role_safe \gset
\if :runtime_role_safe
\else
\echo 'Refusing unsafe runtime role or login-enabled installation owner.'
ROLLBACK;
-- psql \quit ignores an exit status, so a refused script would otherwise
-- leave the process successful after this rollback. ON_ERROR_STOP is set above.
DO $$ BEGIN
    RAISE EXCEPTION 'Refusing unsafe runtime role or login-enabled installation owner.';
END $$;
\endif

-- Optional capability schemas have no baseline runtime authority. Resetting all
-- ACLs for this one runtime role also removes column-level grants from current
-- objects in those schemas without touching their owner or PUBLIC.
REVOKE ALL PRIVILEGES ON SCHEMA core_oauth, core_events, core_console,
    core_charging, core_telegram_auth, core_model, core_image, core_tool_market,
    core_commerce, core_payments, core_humangrant, core_assistant_tools,
    core_assistant, core_market_oauth, core_wallet, core_bounties,
    core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA core_oauth, core_events,
    core_console, core_charging, core_telegram_auth, core_model, core_image,
    core_tool_market, core_commerce, core_payments, core_humangrant,
    core_assistant_tools, core_assistant, core_market_oauth, core_wallet,
    core_bounties, core_trust_pricing FROM :"runtime_role";

-- Web identity is the only optional feature that uses public. Keep its
-- projections scoped; never reset unrelated public objects or baseline schema
-- usage that an operator may have reviewed separately.
REVOKE USAGE ON SCHEMA public FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON public.routing_active_release,
    public.routing_groups, public.routing_routes FROM :"runtime_role";

-- Assistant execution extends the baseline core_gift schema. Its tables and
-- sequence are optional; risk_key, policy, qualifications and lock_eligibility
-- remain baseline grants and are deliberately untouched.
REVOKE ALL PRIVILEGES ON core_gift.decisions, core_gift.registration_profiles,
    core_gift.fingerprints FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON SEQUENCE core_gift.fingerprints_id_seq
    FROM :"runtime_role";

-- Web identity self top-up history: only the receipt columns are optional.
-- Red packets are an optional console capability. Table-level revocation does
-- not remove column UPDATE grants, so reset both before re-provisioning.
REVOKE ALL PRIVILEGES ON core_promotions.red_packets,core_promotions.red_packet_items,
    core_promotions.red_packet_claims FROM :"runtime_role";
REVOKE UPDATE(title,description,cover_image,cover_prompt,draw_mode,per_user_limit,start_at,end_at,
    enabled,updated_at,deleted_at) ON core_promotions.red_packets FROM :"runtime_role";
REVOKE UPDATE(released_at) ON core_promotions.red_packet_items FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON SEQUENCE core_promotions.red_packets_id_seq,
    core_promotions.red_packet_items_id_seq,core_promotions.red_packet_claims_id_seq FROM :"runtime_role";

-- Community history is optional within the baseline history schema. Table-level
-- revocation alone does not remove its explicit column grants.
REVOKE ALL PRIVILEGES ON core_directory_ads.quotes, core_directory_ads.payments,
    core_directory_ads.refunds FROM :"runtime_role";
REVOKE UPDATE(confirmation_digest,confirmation_sid,confirmed_sid,confirmed_at)
    ON core_directory_ads.quotes FROM :"runtime_role";
REVOKE UPDATE(receipt,journal_id) ON core_directory_ads.refunds FROM :"runtime_role";
REVOKE USAGE ON SCHEMA core_directory_ads FROM :"runtime_role";

REVOKE ALL PRIVILEGES ON core_history.connection_snapshot FROM :"runtime_role";
REVOKE SELECT(document,mapping,native_credential_floor,snapshot_observed_at,
    target_database,target_database_oid,target_nonce)
    ON core_history.connection_snapshot FROM :"runtime_role";

REVOKE SELECT(migration_id, manifest) ON core_migration.import_runs FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON core_migration.import_runs FROM :"runtime_role";
REVOKE USAGE ON SCHEMA core_migration FROM :"runtime_role";

-- Account appeal capability: also revoke column grants before re-provisioning.
REVOKE ALL PRIVILEGES ON SCHEMA core_account_appeals FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA core_account_appeals FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA core_account_appeals FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA core_account_appeals FROM :"runtime_role";
REVOKE INSERT(id,user_id,user_version,reason,status,admin_note,reviewed_by,created_at,reviewed_at,revision),
    UPDATE(id,user_id,user_version,reason,status,admin_note,reviewed_by,created_at,reviewed_at,revision)
    ON core_account_appeals.requests FROM :"runtime_role";
REVOKE INSERT(digest,user_id,user_version,proof_kind,expires_at,request_id,created_at),
    UPDATE(digest,user_id,user_version,proof_kind,expires_at,request_id,created_at)
    ON core_account_appeals.flows FROM :"runtime_role";

-- Remove both table and column authority before applying selected capabilities.
REVOKE ALL PRIVILEGES ON FUNCTION public.routing_control_balance_lock() FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON SCHEMA core_channels FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA core_channels FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA core_channels FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA core_channels FROM :"runtime_role";
REVOKE SELECT(id,origin,history_ambiguous,created_at,retired_at) ON core_channels.identities FROM :"runtime_role";
REVOKE INSERT(id,origin,history_ambiguous,created_at,retired_at) ON core_channels.identities FROM :"runtime_role";
REVOKE UPDATE(id,origin,history_ambiguous,created_at,retired_at) ON core_channels.identities FROM :"runtime_role";
REVOKE REFERENCES(id,origin,history_ambiguous,created_at,retired_at) ON core_channels.identities FROM :"runtime_role";
REVOKE SELECT(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version,captured_at) ON core_channels.counter_baselines FROM :"runtime_role";
REVOKE INSERT(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version,captured_at) ON core_channels.counter_baselines FROM :"runtime_role";
REVOKE UPDATE(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version,captured_at) ON core_channels.counter_baselines FROM :"runtime_role";
REVOKE REFERENCES(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version,captured_at) ON core_channels.counter_baselines FROM :"runtime_role";
REVOKE SELECT(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id,recorded_at) ON core_channels.balance_observations FROM :"runtime_role";
REVOKE INSERT(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id,recorded_at) ON core_channels.balance_observations FROM :"runtime_role";
REVOKE UPDATE(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id,recorded_at) ON core_channels.balance_observations FROM :"runtime_role";
REVOKE REFERENCES(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id,recorded_at) ON core_channels.balance_observations FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON core_meta.channel_accounting_upgrades FROM :"runtime_role";

-- Projection capabilities: reset table and explicit column grants.
REVOKE SELECT(request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 INSERT(request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 UPDATE(request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 REFERENCES(request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests)
 ON core_channels.usage_contributions FROM :"runtime_role";
REVOKE SELECT(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 INSERT(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 UPDATE(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests),
 REFERENCES(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests)
 ON core_channels.usage_rollups FROM :"runtime_role";
REVOKE ALL PRIVILEGES ON core_channels.usage_contributions,core_channels.usage_rollups,
    core_channels.projection_dirty,core_channels.projection_state,
    core_meta.channel_projection_upgrades,core_channels.opening_sources,core_channels.opening_baselines FROM :"runtime_role";
REVOKE SELECT(singleton,ready),INSERT(singleton,ready),UPDATE(singleton,ready),REFERENCES(singleton,ready)
    ON core_channels.projection_state FROM :"runtime_role";
REVOKE SELECT(request_id,authorized_at),INSERT(request_id,authorized_at),UPDATE(request_id,authorized_at),REFERENCES(request_id,authorized_at)
    ON core_channels.projection_dirty FROM :"runtime_role";

REVOKE SELECT(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,observed_at,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256,captured_at),INSERT(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,observed_at,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256,captured_at),UPDATE(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,observed_at,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256,captured_at),REFERENCES(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,observed_at,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256,captured_at) ON core_channels.opening_sources FROM :"runtime_role";

REVOKE SELECT(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id,captured_at),INSERT(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id,captured_at),UPDATE(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id,captured_at),REFERENCES(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id,captured_at) ON core_channels.opening_baselines FROM :"runtime_role";



-- Component46, including accidental column grants on private receipts.
REVOKE ALL PRIVILEGES ON core_channels.catalog_observations,core_meta.channel_observations_upgrades FROM :"runtime_role";
REVOKE SELECT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms), INSERT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms), UPDATE(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms), REFERENCES(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms) ON core_channels.catalog_observations FROM :"runtime_role";
REVOKE SELECT(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256,recorded_at), INSERT(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256,recorded_at), UPDATE(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256,recorded_at), REFERENCES(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256,recorded_at) ON core_meta.channel_observations_upgrades FROM :"runtime_role";

COMMIT;
