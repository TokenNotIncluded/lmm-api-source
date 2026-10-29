# Capacity collision recovery

This is the scoped A05 contract and remaining integration boundary. It does not
authorize schema changes, production activity or financial overrides.

For a credited payment P, settled refunds S and currently held refunds H consume
capacity: `P - S - H`. A provider-signed failed refund releases its own hold only
after the Core dispatch binding identifies that exact provider refund. Released
failures consume no capacity. UNKNOWN, timeouts and unsigned provider reads are
not release authority.

If an older released refund A later succeeds while a newer refund B occupies
capacity, preserve A's independently accepted proof in immutable blocked_refunds.
Keep B's reservation and stop new reservations. Do not cancel B, replay its POST,
change either refund ID, replace a proof, or manufacture a success receipt.

When B has a sufficiently bound signed failure and its Core release commits,
the Go worker persists the confirmed release and resumes the original settlement
jobs in the same local transaction. Ordinary host scheduling via `Work` therefore
recovers A without a manual reconciliation call, including after a host restart.
The existing trusted `Service.Reconcile(ctx, orderID)` can also request a retry.
These paths requeue only review-state commit/external_refund jobs with an accepted
proof and the original key. It cannot requeue refund_submit or authorize money.
Core checks the exact original quarantined intent/event/payload/refund/amount,
current capacity and hold binding under its intent lock. It then uses the existing
RefundTopup action, including the existing spent-funds-to-debt policy. Settlement
and its provider_refunds receipt commit atomically. Replays recover that receipt;
other keys/entrypoints cannot debit the same provider refund twice.

The original blocked row remains immutable audit evidence. Its matching
provider_refunds row is the resolution witness. Only unresolved blocked rows
prevent new reservations; the monotonic stale-payment-credit fence stays set.
No public HTTP endpoint or new permission is added for this worker operation.

Still blocked: if B remains pending/UNKNOWN, no authority exists to release it.
If A and B both succeed beyond the original payment, payments.sql caps
refunded_minor at the original amount and ledger.sql rejects RefundTopup beyond
the original credit. Provider-signed individual successes alone cannot establish
whether this represents an actual net over-refund, a reversal or a provider
correction. There is no approved authoritative net settlement/reversal basis,
over-refund loss allocation or dedicated ledger action for this case. Retain the
proofs and unresolved fence; do not delete blocked rows, decrease refunded_minor,
increase the original payment or invent an extra credit/debit. A further reviewed
contract from the funds owner and provider reconciliation is required for that
case. The existing test where B succeeds for the whole payment remains intact.

Evidence boundaries: local synthetic PostgreSQL and real Core/module-worker
processes are developer evidence. They do not certify independent acceptance,
Go host-main wiring, real provider settlement, legacy payments or all A05 flows.
