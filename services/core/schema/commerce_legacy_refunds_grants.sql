-- Root coordinates registration/upgrade and startup ACL checks. No table or
-- column writes: the owner imports grants; only the closed function moves money.
REVOKE INSERT,UPDATE,DELETE,TRUNCATE ON core_commerce.legacy_refund_grants,core_commerce.legacy_refund_operations FROM :"runtime_role";
GRANT SELECT ON core_commerce.legacy_refund_grants,core_commerce.legacy_refund_operations TO :"runtime_role";
REVOKE ALL ON FUNCTION core_commerce.import_legacy_refund_grant(JSONB) FROM :"runtime_role";
GRANT EXECUTE ON FUNCTION core_commerce.finalize_legacy_refund(JSONB,BYTEA,BIGINT,BIGINT) TO :"runtime_role";
