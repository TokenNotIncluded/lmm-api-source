//! Read-only verification of subscription, reward and notification authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    // Resolve through catalog OIDs so an absent schema or missing USAGE is a
    // missing capability, not a name-resolution failure. Effective privileges
    // include grants through PUBLIC and inherited roles.
    let (unsafe_privileges, complete): (bool, bool) = sqlx::query_as(
        r#"
WITH expected_schemas(name) AS (
    VALUES ('core_subscriptions'),('core_promotions'),('core_notifications')
), namespaces AS (
    SELECT n.oid,n.nspname FROM pg_catalog.pg_namespace n
    JOIN expected_schemas e ON e.name=n.nspname
), expected_tables(schema_name,name,can_insert) AS (
    VALUES
      ('core_subscriptions','plans',true),
      ('core_subscriptions','plan_revisions',true),
      ('core_subscriptions','commands',true),
      ('core_subscriptions','quotes',true),
      ('core_subscriptions','purchases',true),
      ('core_subscriptions','refund_quotes',true),
      ('core_subscriptions','external_prices',true),
      ('core_subscriptions','external_orders',true),
      ('core_subscriptions','external_proofs',true),
      ('core_subscriptions','external_grants',true),
      ('core_subscriptions','pancake_prices',true),
      ('core_subscriptions','pancake_orders',true),
      ('core_subscriptions','pancake_events',true),
      ('core_subscriptions','pancake_cycles',true),
      ('core_subscriptions','stripe_prices',true),
      ('core_subscriptions','stripe_state',true),
      ('core_subscriptions','stripe_events',true),
      ('core_subscriptions','stripe_cycles',true),
      ('core_promotions','secrets',false),
      ('core_promotions','campaigns',true),
      ('core_promotions','commands',true),
      ('core_promotions','issues',true),
      ('core_promotions','codes',true),
      ('core_promotions','claims',true),
      ('core_promotions','red_packets',true),
      ('core_promotions','red_packet_items',true),
      ('core_promotions','red_packet_claims',true),
      ('core_notifications','preferences',true),
      ('core_notifications','inbox',true),
      ('core_notifications','price_events',false)
), allowed_updates(schema_name,table_name,column_name) AS (
    VALUES
      ('core_subscriptions','plans','revision'),
      ('core_subscriptions','plans','archived'),
      ('core_subscriptions','quotes','state'),
      ('core_subscriptions','quotes','result'),
      ('core_subscriptions','purchases','canceled_at'),
      ('core_subscriptions','purchases','refund_journal_id'),
      ('core_subscriptions','refund_quotes','result'),
      ('core_subscriptions','external_orders','confirmed_at'),
      ('core_subscriptions','external_orders','canceled_at'),
      ('core_subscriptions','pancake_orders','provider_order_id'),
      ('core_subscriptions','pancake_orders','status'),
      ('core_subscriptions','pancake_orders','event_created'),
      ('core_subscriptions','pancake_orders','event_id'),
      ('core_subscriptions','pancake_orders','paid_period_start'),
      ('core_subscriptions','pancake_orders','paid_period_end'),
      ('core_subscriptions','stripe_state','provider_subscription_id'),
      ('core_subscriptions','stripe_state','provider_customer_id'),
      ('core_subscriptions','stripe_state','status'),
      ('core_subscriptions','stripe_state','event_created'),
      ('core_subscriptions','stripe_state','event_id'),
      ('core_subscriptions','stripe_state','period_start'),
      ('core_subscriptions','stripe_state','period_end'),
      ('core_promotions','campaigns','revision'),
      ('core_promotions','campaigns','title'),
      ('core_promotions','campaigns','budget_units'),
      ('core_promotions','campaigns','spent_units'),
      ('core_promotions','campaigns','reserved_units'),
      ('core_promotions','campaigns','max_claims'),
      ('core_promotions','campaigns','claims'),
      ('core_promotions','campaigns','reserved_claims'),
      ('core_promotions','campaigns','max_claims_per_user'),
      ('core_promotions','campaigns','starts_at'),
      ('core_promotions','campaigns','ends_at'),
      ('core_promotions','campaigns','enabled'),
      ('core_promotions','codes','revoked'),
      ('core_promotions','red_packets','title'),
      ('core_promotions','red_packets','description'),
      ('core_promotions','red_packets','cover_image'),
      ('core_promotions','red_packets','cover_prompt'),
      ('core_promotions','red_packets','draw_mode'),
      ('core_promotions','red_packets','per_user_limit'),
      ('core_promotions','red_packets','start_at'),
      ('core_promotions','red_packets','end_at'),
      ('core_promotions','red_packets','enabled'),
      ('core_promotions','red_packets','updated_at'),
      ('core_promotions','red_packets','deleted_at'),
      ('core_promotions','red_packet_items','released_at'),
      ('core_notifications','preferences','enabled'),
      ('core_notifications','preferences','low_balance_units'),
      ('core_notifications','preferences','below_threshold'),
      ('core_notifications','preferences','revision'),
      ('core_notifications','preferences','updated_at'),
      ('core_notifications','inbox','read_at')
), expected_sequences(schema_name,name) AS (
    VALUES ('core_subscriptions','plans_id_seq'),
           ('core_subscriptions','purchases_id_seq'),
           ('core_promotions','red_packets_id_seq'),
           ('core_promotions','red_packet_items_id_seq'),
           ('core_promotions','red_packet_claims_id_seq'),
           ('core_notifications','inbox_id_seq')
), relations AS (
    SELECT c.oid,n.nspname,c.relname,e.name AS expected_name,e.can_insert
    FROM pg_catalog.pg_class c JOIN namespaces n ON n.oid=c.relnamespace
    LEFT JOIN expected_tables e ON e.schema_name=n.nspname
        AND e.name=c.relname AND c.relkind='r'
    WHERE c.relkind IN ('r','p','v','m','f')
)
SELECT (
    EXISTS(SELECT FROM namespaces n WHERE
        pg_catalog.has_schema_privilege(n.oid,'CREATE,USAGE WITH GRANT OPTION'))
    OR EXISTS(SELECT FROM relations r WHERE
        pg_catalog.has_table_privilege(r.oid,
            'UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR pg_catalog.has_any_column_privilege(r.oid,'REFERENCES')
        OR pg_catalog.has_table_privilege(r.oid,
            'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
        OR pg_catalog.has_any_column_privilege(r.oid,
            'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
        OR (r.can_insert IS DISTINCT FROM true AND (
            pg_catalog.has_table_privilege(r.oid,'INSERT')
            OR pg_catalog.has_any_column_privilege(r.oid,'INSERT')))
        OR (r.expected_name IS NULL AND (
            pg_catalog.has_table_privilege(r.oid,'SELECT')
            OR pg_catalog.has_any_column_privilege(r.oid,'SELECT'))))
    OR EXISTS(
        SELECT FROM relations r
        JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
        WHERE a.attnum>0 AND NOT a.attisdropped
            AND pg_catalog.has_column_privilege(r.oid,a.attnum,'UPDATE')
            AND NOT EXISTS(SELECT FROM allowed_updates permitted
                WHERE r.expected_name IS NOT NULL
                    AND permitted.schema_name=r.nspname
                    AND permitted.table_name=r.relname
                    AND permitted.column_name=a.attname))
    OR EXISTS(
        SELECT FROM pg_catalog.pg_class c JOIN namespaces n ON n.oid=c.relnamespace
        LEFT JOIN expected_sequences e ON e.schema_name=n.nspname AND e.name=c.relname
        WHERE CASE WHEN c.relkind='S' THEN
            pg_catalog.has_sequence_privilege(c.oid,
                'SELECT,UPDATE,USAGE WITH GRANT OPTION,SELECT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
            OR (e.name IS NULL AND pg_catalog.has_sequence_privilege(c.oid,'USAGE'))
            ELSE false END)
    OR EXISTS(
        SELECT FROM pg_catalog.pg_proc p JOIN namespaces n ON n.oid=p.pronamespace
        WHERE pg_catalog.has_function_privilege(p.oid,'EXECUTE')
          AND NOT (n.nspname='core_subscriptions' AND (
            (p.proname='apply_epay_grant' AND pg_catalog.oidvectortypes(p.proargtypes)='text, text, text') OR
            (p.proname='external_ends_at' AND pg_catalog.oidvectortypes(p.proargtypes)='jsonb, bigint')
          )))
) AS unsafe_privileges,
(
    NOT EXISTS(
        SELECT FROM expected_schemas e LEFT JOIN namespaces n ON n.nspname=e.name
        WHERE NOT COALESCE(pg_catalog.has_schema_privilege(n.oid,'USAGE'),false))
    AND NOT EXISTS(
        SELECT FROM expected_tables e
        LEFT JOIN namespaces n ON n.nspname=e.schema_name
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=e.name AND c.relkind='r'
        WHERE NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'SELECT'),false)
            OR (e.can_insert
                AND NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'INSERT'),false)))
    AND NOT EXISTS(
        SELECT FROM allowed_updates permitted
        LEFT JOIN namespaces n ON n.nspname=permitted.schema_name
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=permitted.table_name AND c.relkind='r'
        LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
            AND a.attname=permitted.column_name AND a.attnum>0 AND NOT a.attisdropped
        WHERE a.attgenerated IS DISTINCT FROM ''
            OR NOT COALESCE(pg_catalog.has_column_privilege(c.oid,a.attnum,'UPDATE'),false))
    AND NOT EXISTS(
        SELECT FROM expected_sequences e
        LEFT JOIN namespaces n ON n.nspname=e.schema_name
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=e.name AND c.relkind='S'
        WHERE NOT COALESCE(pg_catalog.has_sequence_privilege(c.oid,'USAGE'),false))
    AND (SELECT count(*)=2 FROM pg_catalog.pg_proc p JOIN namespaces n ON n.oid=p.pronamespace
        WHERE n.nspname='core_subscriptions' AND pg_catalog.has_function_privilege(p.oid,'EXECUTE')
        AND ((p.proname='apply_epay_grant' AND pg_catalog.oidvectortypes(p.proargtypes)='text, text, text')
          OR (p.proname='external_ends_at' AND pg_catalog.oidvectortypes(p.proargtypes)='jsonb, bigint')))
) AS complete
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if !required {
        return Ok(());
    }
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    // Required even when wallet admission is disabled: Stripe subscriptions
    // share original transaction ownership with wallet, legacy and EPay.
    let namespace_complete: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT FROM pg_catalog.pg_namespace n
          JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
          WHERE n.nspname='core_payments' AND c.relname='provider_identifiers' AND c.relkind='r'
          AND pg_catalog.has_schema_privilege(n.oid,'USAGE')
          AND pg_catalog.has_table_privilege(c.oid,'SELECT')
          AND pg_catalog.has_table_privilege(c.oid,'INSERT'))"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if !namespace_complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    // These are shared charging tables. Require the subscription operations'
    // capabilities, while leaving restrictions for other enabled writers to
    // their existing charging checks.
    let charging_complete: bool = sqlx::query_scalar(
        r#"
WITH namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='core_charging'
), required_tables(name,can_insert) AS (
    VALUES ('subscriptions',true),('requests',false),('subscription_opening_consumption',false)
)
SELECT
    COALESCE((SELECT pg_catalog.has_schema_privilege(oid,'USAGE') FROM namespace),false)
    AND NOT EXISTS(
        SELECT FROM required_tables e LEFT JOIN namespace n ON true
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=e.name AND c.relkind='r'
        WHERE NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'SELECT'),false)
            OR (e.can_insert
                AND NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'INSERT'),false)))
    AND COALESCE((
        SELECT pg_catalog.has_column_privilege(c.oid,a.attnum,'UPDATE')
        FROM namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname='subscriptions' AND c.relkind='r'
        JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attname='active'
            AND a.attnum>0 AND NOT a.attisdropped AND a.attgenerated=''
    ),false)
    AND COALESCE((
        SELECT pg_catalog.has_sequence_privilege(c.oid,'USAGE')
        FROM namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname='subscriptions_id_seq' AND c.relkind='S'
    ),false)
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if !charging_complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}
