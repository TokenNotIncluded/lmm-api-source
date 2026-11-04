-- Fresh component 51: preserve both existing, explicit opening identities.
-- Component 50 remains byte-for-byte frozen. No existing-site upgrade is implied.
ALTER TABLE core_payments.legacy_bases
 ALTER COLUMN opening_entity_key DROP EXPRESSION;
ALTER TABLE core_payments.legacy_bases
 ALTER COLUMN opening_entity_key SET NOT NULL;
ALTER TABLE core_payments.legacy_bases ADD CONSTRAINT legacy_basis_opening_identity
 CHECK(opening_entity_key IN ('user:'||source_user_id::text,'legacy.users/'||source_user_id::text));

CREATE OR REPLACE FUNCTION core_payments.guard_legacy_basis() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NOT EXISTS (
   SELECT FROM core_identity.users u
   JOIN core_billing.opening_imports i ON i.account_id=u.personal_account_id
   JOIN core_billing.opening_import_runs r ON r.migration_id=i.migration_id
   WHERE u.id=NEW.user_id AND u.personal_account_id=NEW.account_id
     AND i.migration_id=NEW.opening_migration_id
     AND i.source_entity_key=NEW.opening_entity_key
     AND r.source_archive_sha256=NEW.source_archive_sha256
 ) THEN
   RAISE EXCEPTION 'legacy basis must match original opening and personal owner' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
