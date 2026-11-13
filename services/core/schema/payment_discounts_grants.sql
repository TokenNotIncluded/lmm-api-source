-- Apply as the schema owner after explicit fresh installation or offline upgrade.
-- No extension principal receives coupon mutation or consumption authority.
GRANT SELECT,INSERT,UPDATE ON core_payments.discount_codes TO :"runtime_role";
GRANT SELECT,INSERT,UPDATE ON core_payments.discount_reservations TO :"runtime_role";
GRANT USAGE,SELECT ON SEQUENCE core_payments.discount_codes_id_seq TO :"runtime_role";
