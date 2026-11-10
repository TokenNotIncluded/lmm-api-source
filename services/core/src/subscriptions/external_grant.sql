-- Executed inside the Core SERIALIZABLE transaction, with the same actor lock
-- as wallet confirmation. All writes (grant and durable receipt) commit together.
SELECT core_subscriptions.apply_epay_grant($1,$2,$3)
