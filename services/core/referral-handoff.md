# Imported pending first-cash referral increment

Base: `b69dfe67625970e24ed999ac12e2e2d0f23364f8`. Uncommitted worker changes.
No Go, proto, Web, parent recipe, production or DMIT changes are part of this increment.

## Runtime authority

Fresh components 53 (`referral-ledger-revision`) and 54 (`legacy-referrals`) append
without changing the first 52 components' SQL. Normal startup executes no DDL.
The fresh manifest/catalog pins change; the frozen Core52 binary remains separate.
There is **no supported Core52 -> Core54 installed-site upgrade procedure** here.
Do not apply these fresh components by hand to an existing site or claim an upgrade.

* `PaymentsStore.credit` retains existing signature/proof/identity/ledger checks.
  Its successful `intents.credit_journal_id` update invokes
  `core_referrals.native_payment`, atomically locking the imported contract,
  persisting the first cash source, checking frozen terms and live eligibility,
  and posting one `affiliate_award` through the same journal/entry/balance ledger.
* `PaymentsStore.apply_legacy_payment` invokes `legacy_payment` on successful
  `legacy_applications` insertion. Original frozen micros and credits remain
  separate; no conversion from a display price or original opening is invented.
* Native `PaymentsStore.resolve(commit)` and `external_refund` update the original
  intent's cumulative refund. The same transaction invokes `refund`; partial
  refunds retain the award, full refunds revoke it once. Claimed rewards become
  signed affiliate debt. New awards offset that debt before another claim.
* Ordinary disable/enable has no money hook. Explicit confirmed abuse/appeal uses
  owner-only `core_referrals.moderate`. Actor hierarchy, request identity,
  evidence, source contract, penalty and auth_version are checked in the same
  transaction. Old ban retries cannot undo an appeal. Refunds prevent reward
  restoration; an incorrect abuse penalty can still be restored.
* `scripts/legacy_referral_moderation.py plan/apply` is the owner-controlled local
  management caller. It writes a private preview with exact credit effect, then
  requires its hash and unchanged locked state to apply. It provides **no Web
  administrator route or Go capability**. Runtime cannot execute generic referral
  delta/settle/moderate functions or write the new tables. Existing payment
  runtime grants suffice because only payment-row triggers are security definer.

Provider whitelist: Stripe, Creem, Waffo, Waffo Pancake, EPay CNY only. LDC and
unsupported providers do not consume cash-first eligibility. Low cash, quota,
status, email/Stripe pair, policy or reward-cap failures consume it permanently.
Cash floors use exact integer micros per settlement currency. Registration quota
is an already-posted historical entitlement and is never posted by this import.

## Owner import and source review

`legacy_referral_export.py` emits read-only source SQL. The supplied frozen
`contracts-source.private.json` is also supported directly, without copying it
into this repository. `legacy_referral_prior_cash.py` emits the missing exact
old success/positive-normalized-quota predicate for all 133 earned pending users.
A zero first marker alone is not proof of no older qualifying payment.

`legacy_referral_review.py` derives only the evidenced unchanged-delta case:
source current credit scale is 500000, wallet audit has complete referral_bases,
and neither that plan nor a per-reward rebase names the pending reward. It keeps
old `unrebasedReferralDeltaTx` semantics even if the inviter is in user_ids.
Missing baselines, incomplete prior-cash evidence, or a named pending rebase
reject. No FX or guessed-unit conversion is accepted.

`legacy_referral_import.py` emits an exclusive private SQL plan. Owner review and
application are separate; it never applies automatically. Import is serializable,
bound to original opening/archive/personal identities, rejects nonempty cohorts
and target settlements that predate import, preserves all raw users/bindings,
269 rewards, 472 entries, policies and rebase audits, and activates only the 133
**earned awaiting-first-top-up** contracts. Other 136 historical reward records
remain raw history, including the 134 already-revoked awaiting records and the
empty/null policy records. They are not replayed or made executable by this phase.

The raw policy fallback remains explicit for a hypothetical supported pending
record with absent snapshot; all actual 133 pending snapshots are present.
Historical empty/null snapshots are retained unchanged, not fabricated.

## Evidence and limits

Self-tests use a private PG18.6 Unix-socket cluster on port 55439, synthetic keys
and signatures. Their fixture uses the real opening import function, the Python
contract planner and real Core Store methods. It never inserts a fake settled
journal to certify a payment. Ordinary development tests are not independent
acceptance. Native source guards also require the matching payment journal, provider namespace,
amount, personal account and cumulative refund compensation. Core remains the
trusted provider verifier and proof writer; the SQL trigger does not independently
verify a provider signature. Wrong-source journals and refund-field-only updates
are rejected under the restricted runtime role.

Exact commands, failures, hashes and final outcomes are retained in
`/home/lightjunction/.cache/lmm-api-referral-build/evidence/`.

Pending acceptance/work:

* The supplied prior-cash query confirms zero earlier qualifying orders for all
  133 pending users. A private source review/import plan has been generated.
  Self-test imported all 1239 bindings, 269 raw rewards and 472 raw entries,
  checked opening affiliate sums and zero registration import effects, then
  exercised a source contract through trusted Store cash/tail/full refund.
  Independent acceptance must still reproduce this on its own fresh Core54 DB.
* Installed-site offline upgrade/fencing and rollback are not implemented here.
* Legacy late-payment credit is connected and tested; the pre-existing legacy
  payment subsystem still has no trusted provider-refund ingress/application.
  This increment must not be described as complete legacy-refund migration.
* The 134 historical revoked-awaiting contracts are preserved, not executable;
  restoration of those imported historical cases requires its own reviewed phase.
* New registration referrals, new referral bindings, public history/UI and a Web
  administrator confirmed-moderation flow are outside this existing-pending phase.
* No production, real paid calls, DMIT, commit, push or independent sign-off.
