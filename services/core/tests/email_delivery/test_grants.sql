-- Local fixture only, non-owner role. Explicit columns, no schema/DDL grants.
GRANT SELECT,INSERT ON core_identity.email_delivery_outbox TO email_runtime;
GRANT UPDATE(payload,status,error_class,lease_token,lease_until,smtp_started_at,finished_at)
    ON core_identity.email_delivery_outbox TO email_runtime;
