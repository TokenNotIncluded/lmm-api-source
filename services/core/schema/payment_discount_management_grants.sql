-- Runtime appends management facts; no deletion, rewriting or extension access.
GRANT SELECT,INSERT ON core_payments.discount_code_tombstones,core_payments.discount_management_batches TO :"runtime_role";
