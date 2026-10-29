-- core-26-v5-to-27-v6-console-v2: run after the offline barrier and all old
-- table locks, inside the caller's SERIALIZABLE transaction. No COMMIT here.
-- This is NOT the staged READ COMMITTED email-notice delta.
DO $$ BEGIN
    IF current_setting('transaction_isolation') <> 'serializable' THEN
        RAISE EXCEPTION 'CONSOLE_BRIDGE_SERIALIZABLE_REQUIRED' USING ERRCODE = '55000';
    END IF;
    -- Preserve legacy profile email and email_login_allowed verbatim. The flag
    -- authorizes only the existing alias + password path (identity/browser.rs).
    -- Identity v6 adds email_version=0 and an empty email_addresses table:
    -- neither profile text nor that flag becomes verified reset authority.
    -- Exact v5 has no outbox at all. A staged challenge/notice installation is
    -- a different predecessor, including terminal notices with erased payload.
    IF to_regclass('core_identity.email_delivery_outbox') IS NOT NULL
       OR to_regclass('core_identity.email_addresses') IS NOT NULL
       OR to_regclass('core_identity.email_challenges') IS NOT NULL THEN
        RAISE EXCEPTION 'CONSOLE_BRIDGE_UNREVIEWED_EMAIL_PREDECESSOR' USING ERRCODE = '55000';
    END IF;
END $$;
