# Audio non-streaming integration

This initial slice is integrated in source commit
`3f0e50c9a66f2fdf69d091c1bfb754bc12636ae0`. Its local tests and remaining
scope are recorded in [integration verification](../../../../../docs/integration-verification.md).
The contract below describes the non-streaming entry; it does not claim that
streaming, other audio endpoints or production acceptance are complete.

## Typed integration and authority

- `charging::Price.audio: Option<AudioPrice>` and `Usage.audio: Option<AudioMeteredUsage>`.
  Both use serde default + skip-None. Legacy serialized JSON and its digests do not
  acquire an audio field. Audio counters are disjoint and cannot mix with text,
  cached, image, upstream-cost, tool, grounding or per-request text variants.
- `ReservationBasis::Audio`; `PreparedReservation::audio_chat` pins actual wire,
  route/target/channel/credential, provider revision, request fingerprint and bounds.
  Frozen-price SHA binds all four bounds and aggregate limits, not merely their sums.
- `Price::reservation_ceiling` and `Price::cost` are the charging entry points.
  `AudioPrice` provides their exact rational calculation. The earlier standalone
  `AudioQuote` now delegates to that SAME calculation. The HTTP/PG path never mints
  an independent `AudioQuote` or accepts an independent discount: only the transaction's
  `Price.user_trust` applies. Group/tariff/source revision remain frozen in `Price`.
- `Charging::checkpoint_audio` stores terminal raw usage + response SHA + full frozen
  Price SHA, linked to `route-dispatch:1`, before returning output or settling.
  `finish` and generic checkpoint verify those bytes and the prepared contract;
  a generic `settle/reconcile` call with default usage cannot release an audio hold.
- `Charging::reconcile_audio_receipts` is invoked by `ModelGateway::recover_expired`.
  It loads original prices/dispatch/receipts, never current audio rules. It calls the
  existing reconcile/finish/ledger code, not a new funding writer. Missing terminal
  usage remains held. Ambiguous dispatch is not automatically retried.

## Schema conclusion

No DDL, component fingerprint or new permission is needed for this slice.
Existing `requests.price` JSONB holds the audio price; existing append-only
`usage_events` JSONB holds `audio-terminal:v1`, `usage:audio-terminal:v1`, and final.
Existing SQL token columns remain zero for audio ONLY because typed audio usage is
stored independently, never used as a default zero settlement. Decode reconstructs
and validates the real four counters from immutable events. The existing table
primary keys support receipt lookups. Recovery scanning is bounded to 1..1000 rows.
A future high-volume deployment may need query-plan evidence/index work; none is
introduced here or smuggled into a historical schema component.

## Supported entry and limits

| Surface | Behavior |
| --- | --- |
| POST `/v1/chat/completions`, native API key | Non-streaming, single-choice text/audio under an explicit host AudioRule |
| Authorization | Existing native key + user/group/model/IP checks and charging payer/budget/subscription selection |
| Input | User/system/developer messages, strings, text parts, inline `input_audio` WAV/MP3 |
| Output | Text and/or requested audio with voice/format; explicit `max_completion_tokens` required |
| Usage | Actual upstream `prompt_tokens`/`completion_tokens` plus explicit text/audio details, consistent totals; no missing-counter inference |
| Pricing | Four multiplier weights; fixed USD/request; inclusive total-input volume tiers, exact group and trust factors, explicit rounding |
| Bounds | Reviewed server contract's per-modality AND aggregate input/output maxima; output additionally capped by the request; bytes are never token evidence |
| Retry | Single actual upstream attempt; duplicate request id conflicts without another dispatch |
| Recovery | Persisted terminal receipt can settle after worker loss; partial/unknown usage remains held |
| Streaming, Realtime/WebSocket | Unsupported by the actual entry; no usage synthesized |
| `/v1/audio/speech`, transcriptions/translations | Not registered by this slice |
| Responses/Anthropic/Gemini audio | Rejected by text-entry guards; not converted to text pricing |
| OAuth/session delegation, assistant history audio references, tool calls, images/video | Unsupported by this strict audio entry |
| Cache-discount expressions, arbitrary legacy expressions | Not implemented; v1 counts cached subsets at modality's full rate |

OpenAI's current Chat API reference exposes modality details as optional, so this
entry does NOT promise every OpenAI-shaped provider always supplies them. Configure
only a reviewed provider/model contract that supplies all four counters; otherwise
UNKNOWN holds are intentional. No model-name heuristic, default external-model
bound or synthesized usage is installed. Provider-returned model must equal the
configured upstream snapshot. Official reference consulted without paid calls:
https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create

## Coordinator patch

`/home/lightjunction/.cache/lmm-module-migration/evidence/audio-integration.patch`
supersedes the registration-only first-slice patch (registration is now in source).
It adds omitted `audio: None` in existing callers/tests, guards text no-acceptance
usage, and adds default-empty `ModelConfig.audio_rules` plus `with_audio_rules`.
These out-of-scope changes were not applied by this owner. At final handoff the
shared tree already contains them: full `git apply --reverse --check` passes and
forward check rejects already-present hunks. Do NOT reapply. It does not overwrite whole files.

No new routing protocol is needed: the actual audio entry selects existing OpenAI
Chat routes and their actual credentials/network transport. An AudioRule additionally
pins routing price version, upstream id/model, endpoint, canonical model/group,
source digest, tariff and reviewed bounds. Route publication changes fail closed if
no matching audio rule exists. Core HTTP cannot submit a rule. The old text pricing
catalog does not yet expose this separate audio tariff; public audio pricing UI and
configuration publication require a follow-up and are not accepted here.

## Checks and queued tests

Executed: owned-file rustfmt parse/format check (rustfmt 1.10.0-stable), `git diff
--check`, new-leaf whitespace checks, and `git apply --check` on the coordinator
patch initially PASS. Final shared-tree check finds it already integrated:
forward check fails, full reverse check PASS. MISE_DISABLE_TOOLS=rust used. No Cargo, rustc build, PG or external
model test was run. A read-only initializer scanner was interrupted after its
unbounded prefix search became slow; it was replaced with bounded scanning. It
performed no writes before interruption. A mask missed Rust backslash-newline
strings in its first pass; the corrected pass included charging test initializers.

With the patch content now present, queue under the coordinator's existing toolchain/target
lease (the PG tests require an authorized disposable database fixture):

```
cargo test --manifest-path services/core/Cargo.toml model_http::audio --lib
cargo test --manifest-path services/core/Cargo.toml audio_charging_variant_keeps_legacy_json --lib
```

Supplied tests: prior six exact-pricing/binding unit tests; strict request rejection;
legacy Price/Usage JSON and cross-pricing rejection; three real-PG tests using a
synthetic loopback provider or explicit internal checkpoint fixture:

1. Actual Core HTTP four-counter charge, raw receipt, original price digest,
   duplicate request without second external dispatch, real wallet amount.
2. Missing audio counter leaves the full hold; zero-usage reconciliation rejected.
3. Durable checkpoint followed by object recreation and changed current tariff
   settles original amount once; changed receipt cannot replace the checkpoint.

Test 3 is NOT a real OS restart. Still unrun: compilation, all new tests, restricted
PG results, real process crash/COMMIT ACK-loss cases, final composed candidate,
independent acceptance and external provider compatibility. Do not call this
production ready or claim full audio, full project or deployment completion.
