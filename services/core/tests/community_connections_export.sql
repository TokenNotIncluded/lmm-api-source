-- Run only against the frozen legacy source, with its schema on search_path.
-- No secrets, token IDs or grant IDs are exported. Surviving rows prove positives;
-- these counts do NOT establish absence of hard-deleted credentials or exact firsts.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint AS observed_at,
       current_schema() AS source_schema;
WITH keys AS (
 SELECT user_id,
 count(*) AS token_rows,
 count(*) FILTER (WHERE oauth_managed=false) AS ordinary_rows,
 count(*) FILTER (WHERE oauth_managed=true) AS managed_rows,
 min(created_time) FILTER (WHERE oauth_managed=false AND created_time>0) AS api_key_created_at,
 min(created_time) FILTER (WHERE oauth_managed=true AND created_time>0) AS oauth_token_created_at,
 count(*) FILTER (WHERE created_time IS NULL OR created_time<=0 OR oauth_managed IS NULL) AS unclassified_rows
 FROM tokens GROUP BY user_id
), grants AS (
 SELECT user_id,count(*) AS grant_rows,
 min(created_at_ms) FILTER (WHERE created_at_ms>0) AS oauth_grant_created_at_ms,
 count(*) FILTER (WHERE created_at_ms IS NULL OR created_at_ms<=0) AS unclassified_rows
 FROM oauth_server_grants GROUP BY user_id
), applications AS (
 SELECT user_id,min(created_at) FILTER (WHERE created_at>0) AS application_submitted_at,
 min(reviewed_at) FILTER (WHERE status='approved' AND reviewed_at>0) AS access_approved_at
 FROM developer_access_requests GROUP BY user_id
)
SELECT u.id AS source_user_id,u.created_at AS registered_at,
 k.api_key_created_at,k.oauth_token_created_at,g.oauth_grant_created_at_ms,
 a.application_submitted_at,a.access_approved_at,
 coalesce(k.token_rows,0) AS token_rows,coalesce(k.ordinary_rows,0) AS ordinary_rows,
 coalesce(k.managed_rows,0) AS managed_rows,coalesce(g.grant_rows,0) AS grant_rows,
 coalesce(k.unclassified_rows,0)+coalesce(g.unclassified_rows,0) AS unclassified_rows
FROM users u LEFT JOIN keys k ON k.user_id=u.id LEFT JOIN grants g ON g.user_id=u.id
LEFT JOIN applications a ON a.user_id=u.id ORDER BY u.id;
COMMIT;
