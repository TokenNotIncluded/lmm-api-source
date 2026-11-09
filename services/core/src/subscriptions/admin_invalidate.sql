-- Called only after fresh root/session authorization, original command lookup,
-- and locking the native purchase (if present) followed by its charging grant.
-- $1 grant; $2 target user; $3 actor; $4 credential; $5 operation key;
-- $6 immutable original request; $7 database time; $8 audit reason.
-- No wallet, journal, request, quota-basis or grant lifetime is modified.
WITH revoked AS (
    UPDATE core_charging.subscriptions s SET active=false
    FROM core_identity.users u
    WHERE s.id=$1 AND u.id=$2 AND u.personal_account_id=s.account_id AND s.active
    RETURNING s.id
), stopped AS (
    UPDATE core_subscriptions.purchases p SET canceled_at=coalesce(p.canceled_at,$7)
    FROM revoked WHERE p.subscription_id=revoked.id RETURNING p.id
), receipt AS (
    SELECT jsonb_build_object(
        'contract','core.subscriptions.v1','operation_key',$5::text,
        'subscription_id',revoked.id,'user_id',$2::bigint,
        'invalidated_at',$7::bigint,'renewal_stopped',EXISTS(SELECT FROM stopped)) AS result
    FROM revoked
), command AS (
    INSERT INTO core_subscriptions.commands(actor_user_id,operation_key,request,result)
    SELECT $3,$5,$6,result FROM receipt RETURNING result
), audit AS (
    INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target)
    SELECT $3,$4,'subscription.grant.invalidate',jsonb_build_object('receipt',result,'reason',$8::text)
    FROM command RETURNING 1
)
SELECT command.result FROM command CROSS JOIN audit
