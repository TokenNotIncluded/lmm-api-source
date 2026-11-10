-- Root includes this fragment in the coordinated offline runtime grant path.
-- Registration, manifest/target capture and upgrade versions belong to root.
-- Both table-level and column-level UPDATE must be absent for immutable receipts.
REVOKE UPDATE ON core_commerce.refund_operations FROM :"runtime_role";
REVOKE UPDATE(operation_key,refund_id,order_id,command_sha256,command,actor_user_id,amount_units,amount_native,retired_quantity,journal_id,receipt) ON core_commerce.refund_operations FROM :"runtime_role";
GRANT SELECT,INSERT ON core_commerce.refund_operations TO :"runtime_role";
REVOKE INSERT,UPDATE ON core_commerce.refund_totals FROM :"runtime_role";
REVOKE UPDATE(order_id) ON core_commerce.refund_totals FROM :"runtime_role";
GRANT SELECT ON core_commerce.refund_totals TO :"runtime_role";
GRANT INSERT(order_id), UPDATE(refunded_units,refunded_native,retired_quantity,quantity_basis,quantity_native,revision) ON core_commerce.refund_totals TO :"runtime_role";
GRANT EXECUTE ON FUNCTION core_commerce.post_refund_delta(BIGINT,TEXT,JSONB,BIGINT,BIGINT,BOOLEAN) TO :"runtime_role";
