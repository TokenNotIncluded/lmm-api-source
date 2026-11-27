# Project skills

[Agent guide](../../AGENTS.md) · [Working guide](../../docs/agent-workflows.md)

Read only the matching skill. Add another when work crosses its scope, such as
UI code with new translations. References are not startup reading.

| Skill | Trigger | Not for |
| --- | --- | --- |
| [review-lmm-architecture](review-lmm-architecture/SKILL.md) | Ownership, dependency direction or duplicated decisions | Cosmetic moves without a caller impact |
| [audit-hardcoded-values](audit-hardcoded-values/SKILL.md) | Configuration ignored by a consumer or user-facing literals | Making fixed protocol values configurable |
| [optimize-database-query](optimize-database-query/SKILL.md) | Query amplification, scans and pagination in PostgreSQL | Production database maintenance |
| [repair-regression](repair-regression/SKILL.md) | Reproduce a reported defect and verify the fix with the same input | Broad feature acceptance |
| [lmm-e2e](lmm-e2e/SKILL.md) | Run a complete user journey with browser, services and isolated data | Treating mocks as proof of persistence |
| [docs-maintenance](docs-maintenance/SKILL.md) | Docs, AGENTS.md, skill text, links or examples | Every code edit |
| [create-verification-skill](create-verification-skill/SKILL.md) | Requested automated test, regression or runner/discovery check | CI, production access, paid calls, publication or deployment |
| [deployment](deployment/SKILL.md) | WIP packages, isolated installation or recovery planning | Ordinary source edits without packaging |
| [home-sculpture](home-sculpture/SKILL.md) | Homepage particle scenes, geometry or motion | General UI or static artwork |
| [i18n-translate](i18n-translate/SKILL.md) | UI copy or locale keys, including one-key fixes | Non-UI prose |
| [shadcn-ui](shadcn-ui/SKILL.md) | Components, visual design, forms or task usability | Backend-only work |
| [vercel-react-best-practices](vercel-react-best-practices/SKILL.md) | React request, render or bundle performance | Every React edit |
| [publish-open-source-bounty](publish-open-source-bounty/SKILL.md) | Bounty drafts, prices or publication | Ordinary GitHub issues |
| [review-open-source-bounty](review-open-source-bounty/SKILL.md) | Bounty delivery review or settlement | Ordinary PR review or merge |

Keep the directory and frontmatter `name` identical. Use a short `description`
with clear triggers and exclusions. Keep one task per skill, share check policy
through the working guide, and load large references only when needed. Preserve
upstream attribution; do not create duplicate discoverable `SKILL.md` files.

Every skill inherits the working guide's
[proportion and stopping](../../docs/agent-workflows.md#适度与停止) rules: no
adversarial reviewer will nitpick the code, so no speculative defensive code,
tests only for changed behavior, a bounded number of check runs, and delivery
once the requested result passes its targeted checks.

A skill does not connect tools, provide credentials or authorize an operation.
The bounty skills describe the connected service, not proof that this WIP has
implemented or deployed every corresponding server module.

For skill text, run `python3 -B scripts/check-docs-brand.py --docs-only` once.
Only changes to a helper need that helper's self-tests. The checker does not
verify remote URLs, anchors, actual skill selection or command behavior.
