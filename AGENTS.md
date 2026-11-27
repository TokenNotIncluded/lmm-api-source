# Agent guide

## Project importance and completion

The owner values this project above USD 10,000. Protect funds, permissions, user data and recovery evidence. Keep changes small enough to verify.
For project completion, implement the [product requirements](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md) and independently verify the [production acceptance contract](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md). Record outstanding work in [architecture progress](docs/progress.md).

## Start once

Use the existing `main` checkout in `TokenNotIncluded/lmm-api-source`. Check the revision, local changes and remote head. The parent `TokenNotIncluded/lmm-api` owns the recipes and pins the source. Compare the old monolith at `go-v0.2.118`. [Branch and gitlink policy](docs/development.md#仓库与主线)
Authorized developers may commit and push to the same `main`. Agree on the order of changes when file scopes overlap. Preserve other contributors’ changes and use fast-forward pushes in the existing worktree.
Use the parent repository’s Git-ignored `out/coordination/tasks/` for active work. Record the goal, claimed files, base SHA, status and delivered SHA. Each participant writes a separate reply or atomically updates their own record.
On a cross-machine handoff, have the recipient read back the actual file and revision. Keep task records separate from credentials, security settings and `.git`.

## Pending cross-repository work

Read the [Extore integration contract](docs/extore.md) before synchronizing products or updating its submodule pointer. Preserve both parties’ authorization. Do not deploy Extore or merge or push its `main` branch, which deploys automatically.

## Find the work

| Task | Entry |
| --- | --- |
| Locate code, tools or a business contract | [Documentation index](docs/README.md) · [Development](docs/development.md) · [Working guide](docs/agent-workflows.md) · [Abuse controls](docs/abuse-controls.md) |
| Review architecture, duplication, hardcoded values or SQL | [Architecture review](.agents/skills/review-lmm-architecture/SKILL.md) · [Hardcoded values](.agents/skills/audit-hardcoded-values/SKILL.md) · [Query optimization](.agents/skills/optimize-database-query/SKILL.md) |
| Repair a defect or verify a user journey | [Bug repair](.agents/skills/repair-regression/SKILL.md) · [Automated checks](.agents/skills/create-verification-skill/SKILL.md) · [End-to-end tests](.agents/skills/lmm-e2e/SKILL.md) |
| Change UI, rendering or performance | [UI](.agents/skills/shadcn-ui/SKILL.md) · [Homepage scenes](.agents/skills/home-sculpture/SKILL.md) · [React performance](.agents/skills/vercel-react-best-practices/SKILL.md) |
| Edit docs, package a release or handle a bounty | [Documentation](.agents/skills/docs-maintenance/SKILL.md) · [Deployment](.agents/skills/deployment/SKILL.md) · [Project skill index](.agents/skills/README.md) |

## UI copy and i18n

Follow the [translation workflow](.agents/skills/i18n-translate/SKILL.md) for every change to user-facing text. Use `react-i18next`, flat English-source keys and all seven supported BCP 47 locales. Apply translation patches with the repository helper and review the synchronized translations.

## Reuse upstream evidence

For a new or replacement Rust Core crate, verify the upstream revision, license, current stars in its field and fit for the specific behavior. Retain the owner’s choices of zconv, zhex, zclap, sysinfo, tokio-util and serde_json. [Dependency decisions](docs/rust-dependencies.md)
For difficult changes, use `gh` to inspect maintained implementations and issues. Verify external code against the project contract and preserve attribution. Research grants no CI, publication or deployment authority.

## Branch constraints

Rust Core owns identity, authorization, funds, transactions and model traffic and runs without Go. Go Extensions owns extension records and has no Core database or spending authority. The CLI is a pinned submodule and the Web ships static assets. Preserve the three-program architecture. [Ownership boundaries](docs/core-migration.md)
Use isolated databases and private test credentials. Keep installation-owner and runtime credentials separate. Ordinary startup must never initialize or upgrade a database. Existing sites require a reviewed upgrade procedure. Never register production accounts for tests or rewards.

## Check and finish

Run tests, builds and browser checks on a cloud host or GitHub-hosted runner. Keep these workloads off the user’s desktop and development machine. Choose the smallest check that exercises the changed behavior. [Check selection and stopping rules](docs/agent-workflows.md#适度与停止)
For routine changes, report the developer’s self-test and whether independent acceptance ran. For requested acceptance, freeze the inputs and have a different tester exercise the real business path, including persistence and permission checks where relevant. Retain failures and record unrun or blocked cases. [Quality and handoff](docs/quality.md#测试与开发分工)
Report the tested revision, command, outcome and remaining limits. Keep push, merge, release and production deployment separately authorized and verified. Preserve financial previews and confirmations, licenses and attribution. Read back remote writes before retrying an uncertain operation. [Contribution and delivery](docs/agent-workflows.md#贡献与交付)

<!-- OPENWIKI:START -->

## OpenWiki

This repository has a generated `openwiki/` evidence index. It is optional just-in-time context, not required startup reading.

- Do not enumerate, preload, or search wikis at task start. Use retrieval when the user asks for it, when unfamiliar architecture or dependency behavior materially affects the task, or when source inspection leaves an important uncertainty. Stop once the question is grounded.
- When those conditions apply and OpenWiki retrieval tools are available, use `openwiki_search` for just-in-time context and `openwiki_read` for the relevant complete sections. If search returns `workspace_required`, ask which listed workspace to use and retry with its ID.
- Use `openwiki_list_workspaces` or `openwiki_list_wikis` when workspace membership itself needs to be discovered.
- If the retrieval tools are unavailable, read `openwiki/quickstart.md` and follow its links to the relevant pages.
- Treat source code and tests as authoritative. A brief's unknowns and review items are verification gaps, not automatic requirements.
- Prefer the narrowest quiet validation that proves the changed behavior. Preserve complete failure output.

The scheduled OpenWiki GitHub Actions workflow refreshes the repository wiki. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->
