-- Read-only projection. $1 is one database time captured for the whole page.
-- Imported grants retain their own identity and quota basis; a matching title,
-- time or amount is never sufficient evidence of a native plan association.
WITH records AS MATERIALIZED (
    SELECT s.id,u.id AS user_id,coalesce(profile.username,'') AS username,
           coalesce(profile.email,'') AS email,s.grant_reference,
           CASE WHEN purchase.id IS NOT NULL THEN 'wallet'
                WHEN starts_with(s.grant_reference,'legacy-sub:') THEN 'imported'
                ELSE 'other' END AS source,
           CASE WHEN purchase.id IS NOT NULL THEN jsonb_build_object(
               'id',quote.plan_id,'revision',quote.plan_revision,
               'archived',plan.archived,'terms',revision.terms) ELSE NULL END AS plan,
           s.anchor AS starts_at,s.ends_at,s.active,
           CASE WHEN purchase.refund_journal_id IS NOT NULL THEN 'refunded'
                WHEN s.ends_at<=$1 THEN 'expired'
                WHEN NOT s.active THEN 'invalidated'
                WHEN s.anchor>$1 THEN 'scheduled' ELSE 'active' END AS status,
           purchase.id AS purchase_id,purchase.refund_journal_id,
           CASE WHEN purchase.id IS NOT NULL THEN purchase.canceled_at IS NOT NULL END AS renewal_canceled
    FROM core_charging.subscriptions s
    JOIN core_identity.users u ON u.personal_account_id=s.account_id
    LEFT JOIN core_identity.user_profiles profile ON profile.user_id=u.id
    LEFT JOIN core_subscriptions.purchases purchase ON purchase.subscription_id=s.id
    LEFT JOIN core_subscriptions.quotes quote ON quote.id=purchase.quote_id
    LEFT JOIN core_subscriptions.plan_revisions revision
           ON revision.plan_id=quote.plan_id AND revision.revision=quote.plan_revision
    LEFT JOIN core_subscriptions.plans plan ON plan.id=quote.plan_id
    WHERE ($3::bigint IS NULL OR u.id=$3)
      AND ($4::bigint IS NULL OR quote.plan_id=$4)
      AND ($2::text='' OR strpos(lower(u.id::text||' '||coalesce(profile.username,'')||' '||
           coalesce(profile.email,'')||' '||coalesce(revision.terms->>'title','')||' '||s.grant_reference),lower($2))>0)
), filtered AS MATERIALIZED (
    SELECT * FROM records WHERE $5::text='all' OR status=$5
), page AS (
    SELECT * FROM filtered ORDER BY id DESC LIMIT $6::bigint OFFSET $7::bigint
), projected AS (
    SELECT page.*,quota.start_at AS window_start,quota.end_at AS window_end,
           quota.limit_credits::text AS limit_units,quota.unlimited,
           (coalesce(opening.consumed_credits,0)+coalesce(usage.used,0))::text AS used_units,
           coalesce(usage.reserved,0)::text AS reserved_units
    FROM page
    LEFT JOIN core_charging.subscription_opening_consumption imported ON imported.subscription_id=page.id
    LEFT JOIN LATERAL core_charging.subscription_window(
        page.id,greatest(page.starts_at,coalesce(imported.start_at,page.starts_at),least($1,page.ends_at-1))) quota ON true
    LEFT JOIN core_charging.subscription_opening_consumption opening
           ON opening.subscription_id=page.id AND opening.start_at=quota.start_at
    LEFT JOIN LATERAL (
        SELECT sum(CASE WHEN r.state='settled' THEN r.settled-r.refunded ELSE 0 END) AS used,
               sum(CASE WHEN r.state IN ('reserved','streaming','reconcile') THEN r.reserved ELSE 0 END) AS reserved
        FROM core_charging.requests r WHERE r.source_subscription_id=page.id AND r.source_start=quota.start_at
    ) usage ON true
)
SELECT jsonb_build_object(
    'contract','core.subscriptions.v1',
    'items',coalesce((SELECT jsonb_agg(to_jsonb(projected) ORDER BY id DESC) FROM projected),'[]'::jsonb),
    'total',(SELECT count(*) FROM filtered),'page',$8::bigint,'page_size',$6::bigint)
