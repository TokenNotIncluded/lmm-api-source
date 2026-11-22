-- Explicit owner upgrade: grant only objects created by component 81.
-- Every ACL on a preexisting schema, table, column, sequence and function must
-- remain byte-for-byte unchanged. A standard full Core80 runtime already has
-- its payment, promotion, pending-ledger and Core79 reset readers. Feature-
-- specific deployments retain their previous capabilities; enabling another
-- feature is a separate explicit runtime-role configuration operation.
GRANT SELECT ON core_promotions.legacy_red_packet_bases,core_promotions.legacy_red_packet_items,
    core_promotions.legacy_red_packet_claim_links TO :"runtime_role";
GRANT INSERT(claim_id,item_id) ON core_promotions.legacy_red_packet_claim_links TO :"runtime_role";
GRANT UPDATE(released_at) ON core_promotions.legacy_red_packet_items TO :"runtime_role";
GRANT SELECT ON core_payments.legacy_discount_bases,core_payments.legacy_discount_holds,
    core_payments.legacy_discount_owners,core_payments.legacy_discount_hold_state,
    core_payments.discount_code_usage TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_payments.lock_legacy_discount_code(BIGINT) TO :"runtime_role";
