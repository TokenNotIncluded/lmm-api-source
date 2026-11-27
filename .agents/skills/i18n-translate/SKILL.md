---
name: i18n-translate
description: "Add or correct UI copy and locale keys in apps/web, including one-key fixes. Preserve all locales and interpolation. Not for README or other non-UI prose."
---

# Frontend translations

Follow the changed call site to the resource that supplies its text. Preserve the
flat key, brand names, identifiers and `{{variables}}`. Use [the glossary](../../../docs/translation-glossary.md)
for terminology. Never copy task instructions into translation values.

### Feature-owned translations

When a feature registers its own resources with `addResourceBundle`, edit the
existing feature map for each supported language. HeroSMS, for example, registers
[its translations](../../../apps/web/src/features/email-activations/i18n.ts) when
the feature loads. The shared JSON helper and sync script only cover the locale
files. For a HeroSMS change, run its registration test from the source root:

```bash
(cd apps/web && mise exec -- bun test --preload ./scripts/test-preload.mjs ./src/i18n/feature-translations.test.ts)
```

This test checks keys and placeholders in all seven languages and looks up the
registered text in Simplified and Traditional Chinese.

### Shared locale files

Keep shared strings under `translation` in `apps/web/src/i18n/locales/*.json`.
Register configuration-driven keys passed through variables in
[`STATIC_I18N_KEYS`](../../../apps/web/src/i18n/static-keys.ts) when the extractor
cannot see a literal call. Sync compares locale files and cannot detect a new
key missing from every file.

Create a temporary JSON patch outside the repository, with every supported
locale and the same changed keys in each. Values must be real, non-empty
translations. To remove a retired key, use JSON `null` for that key in every locale. Use [the existing helper](scripts/apply-translations.mjs):

```bash
mise exec -- node .agents/skills/i18n-translate/scripts/apply-translations.mjs /tmp/lmm-translations.json
mise exec -- bun run --filter @lmm/web i18n:sync
```

Use the actual patch path. Apply validates locales and placeholders before
writing; a separate `--check` is for a requested preview, not required before
applying the same patch. Do not copy a writer or manually edit locale JSON.
Writes are sequential, not atomic across files; inspect a partial failure first.

Review every changed locale and generated `_reports/_sync-report.json` and
`_extras` output. Preserve unrelated changes. Sync is a write operation, not a
translation service: it chooses the locale with the most leaf keys as its base,
fills missing values and moves extras. Zero missing keys does not prove correct
translation. Read [the implementation](../../../apps/web/scripts/sync-i18n.mjs)
only when needed; do not hand-edit reports to conceal errors.

For value-only changes, validate the patch and final locale diff. Call-site or
type changes also need affected tests and `mise exec -- bun run typecheck:web`.
Check affected long-text/mobile layouts when text changes layout. No full Web
build is needed solely for JSON text. Report missing language/browser checks.
Only helper-code changes require its `node --test` self-test suite.

For shared locale changes, finish with complete keys across locales, preserved variables,
reviewed sync output and reported checks. Delete only your own temporary patch.

No adversarial reviewer will nitpick these strings; follow [proportion and stopping](../../../docs/agent-workflows.md#适度与停止).
Apply the patch once, review the diff and stop. Do not add extra validation
scripts, retranslate unchanged keys or rerun sync on unchanged input.
