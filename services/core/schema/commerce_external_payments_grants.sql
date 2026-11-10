-- Draft first-slice grants. Not a Core62 change or component registration.
GRANT USAGE ON SCHEMA core_commerce_external TO :"runtime_role";
GRANT SELECT ON core_commerce_external.merchant_channels TO :"runtime_role";
GRANT SELECT,INSERT ON core_commerce_external.orders,core_commerce_external.payment_bases,core_commerce_external.refunds TO :"runtime_role";
GRANT UPDATE(state,submit_count,dispatch_fence) ON core_commerce_external.refunds TO :"runtime_role";
-- No direct ledger writes, income receipt binding, execution-proof assertion,
-- merchant ownership edits, deletion, truncate, or financial-completion update.
