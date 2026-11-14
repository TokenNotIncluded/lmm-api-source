---
name: 'LMM Forge Console'
colors:
  background: 'var(--background)'
  surface: 'var(--card)'
  text: 'var(--foreground)'
  primary: 'var(--primary)'
---

# Design System: LMM Forge Console

## Visual Theme & Atmosphere

- [observed] Authenticated product surfaces use a restrained, Google-inspired console with open spacing, neutral tonal surfaces, semantic accents, and theme-aware navigation. The default light canvas is white with pale gray working surfaces; dark mode uses deep charcoal with lighter surfaces. Shared page cards have no decorative border or shadow.
- [observed] Authentication uses the active semantic palette and a centered, single-column form. Public scenes retain their declared visual exceptions; their editorial palettes do not become defaults for operational pages.
- [design rule] Make the current task easy to scan through alignment, typography, and space. Give related controls one clear group; let secondary history and metadata recede without surrounding each region with a frame.

## Color Palette & Roles

- [observed] Components consume semantic variables: `--background`, `--foreground`, `--card`, `--muted`, `--primary`, `--border`, `--success`, `--warning`, `--info`, `--destructive`, and their foreground pairs.
- [observed] Both light and dark modes define complete semantic roles. State is never encoded by a literal page-local color alone.
- [observed] Default canvas/card pairs are `#ffffff` / `#f7f8fa` in light mode and `#111214` / `#1b1d21` in dark mode. Components consume the semantic variables so named palettes and user preferences remain effective.
- [observed] Forge raw identity tokens use warm paper/ink, cactus, clay, and sage values, but they are mapped through shared theme variables before components consume them.
- [inferred confidence=high] Third-party providers do not introduce their own palette. Use a provider mark only when an approved shared identity token already exists.

## Typography Rules

- [observed] The default authenticated body face is Public Sans through `--font-body`; the optional Lora editorial axis belongs to declared editorial presets, not routine settings or utility screens.
- [observed] Console page titles use `clamp(1.5rem, 2.5cqi, 2rem)`, weight 600, a 1.25 line height and tight tracking. Their scale follows the available content container, and long titles can wrap. Shared card titles use medium weight; page and section headings have no decorative cut marker.
- [inferred confidence=high] Let a clear title and readable text measure lead the page. Use tabular numerals for comparable values and generous gaps between sections; reserve display/serif treatments for established editorial scenes and explicit typography preferences.

## Component Stylings

- [observed] shadcn/ui is configured as `base-luma`, neutral, CSS-variable driven, with Hugeicons and an inverted subtle menu.
- [observed] Existing primitives carry hover, focus, disabled, error, loading, light/dark, and reduced-motion behavior. Prefer them over page-local replacements.
- [observed] `Card` defaults to the `paper` variant: a tonal surface with no border, ring or shadow. Shared card padding is 20px on compact screens and 28px from `sm`; small cards retain 16px padding. The explicit `outlined` variant remains available where a boundary serves the interaction.
- [observed] Settings forms use a two-column desktop grid, full-span switch/textarea rows, shared-radius grouped controls, and a consistent vertical rhythm.
- [observed] Error states combine a named icon, title, optional description, and an explicit retry/action; toast is not the only recovery path.
- [inferred confidence=high] Cards represent true grouped tools or independently actionable objects, not every row or section. Badges represent state/count only; filters use controls with real selection semantics.

### Component foundation

Luma recipes are applied to the owned primitive source files on the Base UI version pinned in [package.json](package.json). Preserve existing exports, custom sizing, semantic colors, keyboard behavior, menu event adapters and page-level class overrides. `src/components/ui/luma-migration.json` records the component inventory and upstream CSS checksum; `src/components/ui/LUMA-LICENSE.txt` preserves source attribution.

Drawer composes Base UI Portal, Backdrop, Viewport, Popup, Content and VirtualKeyboardProvider. Its trigger uses `render` instead of `asChild`. OTP renders actual Base UI inputs; its authentication consumer maps React Hook Form's value/onChange/ref, keeps numeric validation and backup-code mode, and sets `autoSubmit={false}`. OTPField is project-specific rather than the Luma registry's input-otp recipe. `vaul` and `input-otp` are not direct runtime dependencies.

TanStack Table, React Hook Form, cmdk, Sonner and React Day Picker retain their own roles. Primitive updates do not replace charts, the editor, brand icons, the token cloud or every business layout. Preserve 44px native selects on phones, controlled multiline height, paper cards, line/vertical tabs and reduced-motion support. Component changes must preserve access permissions, payment, price locks and login endpoint behavior.

### Component review

Run from `apps/web`:

```sh
LMM_ENABLE_PERSONA_DEBUG=1 bun run dev --port 4174
node scripts/ui-foundation-review.mjs
```

The loopback-only `/?ui_review=1` component gallery uses a separate debug entry and is not imported by production routes. The review script exercises tabs/drafts, switch geometry, OTP paste/validation/explicit submission, nested dialog select, drawer input/Escape/focus restoration and reduced motion at 1440, 834, 390 and 320 pixels. Set `CONSOLE_REVIEW_OUTPUT` for artifacts; `PLAYWRIGHT_MODULE` may point to an installed Playwright module.

Choose the console page, interaction or navigation script for the changed behavior, following the [working guide](../../docs/agent-workflows.md#按范围选择检查). The gallery blocks external/API traffic and uses synthetic local data. It does not sign in to production, make payments, buy numbers or change management settings; its screenshots do not establish production service health.

## Layout Principles

- [observed] The authenticated shell is height-bounded, safe-area aware, sidebar-based at desktop widths, and reserves bottom navigation space on compact screens. At viewport heights of `36rem` or less, section pages scroll their heading, content and footer together so chrome cannot consume the whole content area.
- [observed] `SectionPageLayout` aligns its heading, content and optional footer through the same `--page-inset`. This inset combines a `clamp(1rem, 4cqi, 3rem)` gutter with an `80rem` maximum content width; individual forms may use a narrower reading measure inside it.
- [observed] Heading and action rows wrap according to the actual workspace left after navigation and the assistant. At content-container widths of `40rem` or less, actions occupy their own row and align to the content start. Action labels may wrap, and buttons can grow vertically.
- [observed] Shared settings forms become two columns at `lg`; mobile content remains a single readable column with 16px inputs to avoid focus zoom.
- [observed] Authentication uses `min-h-dvh` and natural document scrolling. The centered form has a `25rem` maximum width; the optional arcade remains in a footer disclosure on every screen size.
- [design rule] Give shrinking grid and flex children `min-width: 0`; wrap long labels and values at the owning component. Keep tables or code that require horizontal comparison inside a local scroll region. Their width must not force the surrounding page to scroll sideways.
- [design rule] Separate sections with space and, where needed, a quiet surface or one divider. Reserve visible boundaries for controls and meaningful groups; do not stack decorative frames.

### Developer and wallet workspaces

The developer page uses a sticky reference navigation column beside its integration workspace. At `62rem` or less in the `developers` container, navigation becomes a wrapping row above the workspace. A copyable API base URL precedes the request builder; protocol tabs span the builder. Model/prompt fields sit beside the code preview when the inner `developer-content` container reaches `44rem`, and stack below that width. Endpoint rows stack at `32rem` or less in the same inner container. These breakpoints follow usable workspace width rather than the browser viewport. Language selection and copying sit in the preview header. Fine rules separate workspace groups and endpoint rows, while the pricing and OAuth references retain their source links and expandable integration prompts.

The wallet separates balance, payment, subscriptions and rewards with wider gaps. Balance and quote figures use larger tabular numerals; the quote groups the exact charge, discount code and wallet credit on one muted surface. Payment methods, quote review, explicit confirmation, order recovery and status refresh retain their existing behavior. Visual grouping does not add financial data or authorize a payment.

### Reflow and zoom review

Reflow checks must record the mechanism being tested. A narrow viewport, native browser zoom, and a root-font override exercise different layout constraints; one result does not establish the others. [WCAG 1.4.4](https://www.w3.org/WAI/WCAG22/Understanding/resize-text.html) requires usable text resizing up to 200%, while [WCAG 1.4.10](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html) specifies reflow at 320 CSS pixels for vertically scrolling content, with exceptions for content that needs a two-dimensional layout.

The [layout review script](scripts/layout-reflow-review.mjs) defines the following checks. These are execution targets, not a record of passing results.

| Method              | Scenarios                                                              | Evidence to inspect                                                                                            |
| ------------------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Viewport sizing     | 1440×900, 390×844, 320×740, 844×390; light mode plus dark phone layout | Shared left edges, local rather than document overflow, wrapped controls, and reachable content/footer actions |
| Native browser zoom | 125%, 150%, 175%, 200%; 400% from a 1280-pixel-wide window             | Requested and read-back zoom factor, reduced CSS viewport, screenshots and focus/reachability results          |
| Root font size      | Twice the initial computed root size at 1440×900                       | Text and controls can grow without clipping; report this as a root-font override, not browser text-only zoom   |

The script covers API keys, users, wallet, developers, system information settings, and sign-in with isolated local fixtures. It checks initial and expanded filter layouts, content/header/footer alignment, horizontal overflow, and whether ending controls can be scrolled into view and focused. Native zoom uses a temporary Chromium extension through [`chrome.tabs.setZoom`](https://developer.chrome.com/docs/extensions/reference/api/tabs#method-setZoom), reads the factor back, and checks the resulting CSS viewport. It does not substitute CSS transforms or device-pixel-ratio changes for browser zoom.

Run the review from `apps/web` with separate debug and production-preview servers. Start the debug server on port 4174 as shown above. In a second terminal:

```sh
bun run build && bun run preview --port 4175
```

With both servers running, use Playwright 1.56.1 and its installed Chromium build:

```sh
LAYOUT_REVIEW_OUTPUT=/tmp/lmm-layout-review node scripts/layout-reflow-review.mjs
```

`LAYOUT_REVIEW_OUTPUT` is required. Set `PLAYWRIGHT_MODULE` to the installed Playwright module when it is outside the project. `LAYOUT_CONSOLE_ORIGIN` and `LAYOUT_PUBLIC_ORIGIN` may select different loopback ports; the script requires distinct `http://127.0.0.1` origins. The [Layout reflow review workflow](../../.github/workflows/layout-reflow-review.yml) prepares these dependencies and servers and retains screenshots, logs, the source revision, and `report.json` even when a check fails.

Review the saved screenshots as well as the measured results. Report each executed scenario and any failures against its tested revision; an unrun scenario stays unverified. These fixtures establish local layout evidence, not production login/payment behavior or complete accessibility conformance.

## Motion & Interaction

- [observed] Loading may use the shared skeleton shimmer; page and state transitions remain restrained.
- [observed] Focus-visible controls expose a clear semantic ring, interactive elements expose pointer affordance, and fixed chrome accounts for safe areas.
- [inferred confidence=high] Animate only named state changes with short opacity/transform transitions, preserve layout stability, and honor reduced motion. Never use motion to decorate financial or status changes.

## Accessibility

- [observed] The authenticated layout includes a skip-to-main control and semantic layout regions.
- [observed] Shared feedback primitives pair icons with text and explicit actions.
- [inferred confidence=high] Every icon-only action needs an accessible name and tooltip where recognition is not universal. Status requires text/icon in addition to color.
- [inferred confidence=high] Preserve keyboard reachability, focus after dialogs/sheets, comfortable compact-screen hit targets, and readable light/dark contrast.

## Frontend behavior boundaries

These rules describe retained browser behavior, not completed server integration. The current core HTTP host and extension host do not expose the legacy assistant, tool-market, store or ranked-game APIs. Pages, mocks, fixtures and browser receipts cannot establish backend availability, payment settlement or production acceptance. Current server contracts live in the module READMEs, including [tool market](../../services/extensions/internal/modules/toolmarket/README.md); financial safeguards belong to the [payment contract](../../docs/payment-money-contract.md).

### Navigation, motion and optional experiences

- Keep the homepage directory entry a real link with click and Enter fallback. Wheel/swipe progress is bounded; reverse movement, multiple touches and interactive controls cancel the gesture. Remove owned listeners on unmount.
- Use one mobile scroll-chrome provider per layout. Menus, dialogs, text editing and short pages remain reachable; route/keyboard changes reset hiding. Hidden chrome is inert and releases reserved height. Respect explicit scroll roots, ignore markers and reduced motion.
- Waiting companions are optional local play after four seconds. They never delay output/navigation, send network requests, award rewards or move money. Reset per task, preserve 48px controls, accessible status and restored focus; collapsing play does not cancel the task.
- Shared shader surfaces use at most one GPU context, bounded initial resolution and frame rate, offscreen/hidden pausing and a static reduced-motion fallback. Preserve generated-source attribution/checksums and composition sync checks. They must not obstruct controls or imply financial status. The homepage poster retains its separately documented rendering budget.
- Signal Path is optional play, never an authentication or anti-bot gate. Agent identity is self-reported. Publishing requires the signed-in user and explicit confirmation; WebMCP state excludes claim tokens and private email. A local game or submission UI does not establish ranked-server availability.

Sources: `src/features/forge/home-directory-gesture.ts`, `src/components/layout/lib/mobile-scroll-controller.ts`, `src/components/wait-companion.tsx`, `src/components/shaders/shader-policy.ts`, `src/features/signal-game/webmcp.ts` and their adjacent tests.

### Assistant and tool consent

- OpenUI accepts only its bounded Stack, Metric, BarChart, DataTable and ConsoleLink catalog. No arbitrary HTML, script, forms or network actions. Keep units consistent, distinguish missing values from zero, retain Markdown explanation and show source fallback on invalid output. Streaming content is noninteractive; a navigation link does not complete a business action.
- Preference actions are bound to the current user/session and run at most once. Mode/theme previews expire without saving; language/currency patch only their fields, and currency changes display only. Undo touches matching fields from the latest action and preserves later manual changes. Prepared actions are not successful receipts; failed or uncertain writes are not replayed.
- Human handoff scopes requests/polling to the visible user, session and conversation and discards stale responses. Takeover pauses AI and disables old operation cards. A saved request does not promise a human is available. Administrative actions keep their actual confirmation and receipt paths, including explicitly validated chat confirmation where implemented; ambiguity does not authorize replay.
- Tool-policy edits remain drafts until saved. Group disable wins; unavailable policy never enables tools. Gate advanced rules on advertised capabilities, preserve drafts on conflicts and require review instead of silent retry or broader permissions.
- Connection secrets stay in transient account-scoped component state, outside caches and JSON previews. Copy requires explicit action; account/provider switches, hiding, expiry or revocation clear them, with a five-minute display limit. Failed permission reads are failures, not empty grants. Failed delegation preserves the original once-only token instead of minting a replacement. Disconnecting does not cancel or refund accepted work.
- Login and installed connections do not authorize paid calls. Consent names the client, tool version, permissions, price ceiling, finite budget and expiry; unsupported quotes are never shown as free. Preserve exact integers and raw business JSON through editing and locale changes. Browser validation does not replace server authorization.
- A submitted call freezes parameters and request identity. Confirm each paid round; after network errors or unknown/unsettled results, query or retry that same identity. Only an explicit refusal before execution permits edited parameters and a new identity. Show immutable pricing/usage evidence; provider-reported or unknown usage is not platform-verified.

Sources: `src/components/ai-elements/openui/policy.ts`, `src/features/assistant/use-assistant-ui-preferences.ts`, `use-assistant-support.ts`, `src/features/system-settings/content/assistant-tool-policy-editor.tsx`, and `src/features/tool-market/{connections.tsx,service-editor.tsx,invoke-body.ts,call-utils.ts}` with their focused tests. UI presets, old MetaMCP descriptions and paid-test examples do not establish supported server capabilities.

This section consolidates the useful browser constraints from sixteen former guides in the [fixed source revision](https://github.com/TokenNotIncluded/lmm-api/tree/c33503f212cbcfab0695d554f57b5153cd4f3bfd/docs): `frontend-design`, `home-directory-navigation`, `mobile-scroll-chrome`, `waiting-experience`, `shaders`, `games/signal-path`, `assistant-openui`, `assistant-ui-preferences`, `assistant-human-support`, `assistant-tool-policy`, `tool-market-connections`, `tool-market-guide`, `tool-market-metamcp`, `tool-market-metering`, `tool-market-paid-test-mcp` and `tool-market-provider-presets`. Historical commands and deployment claims remain history; current source and versioned contracts determine behavior.

## Source references

- `components.json`
- `src/styles/index.css`, `src/styles/theme.css`, `src/styles/forge-tokens.css`
- `src/styles/console-editorial.css`, `src/styles/console-pages.css`
- `src/components/ui/card.tsx`
- `src/components/layout/components/authenticated-layout.tsx`
- `src/components/layout/components/section-page-layout.tsx`
- `src/features/system-settings/components/settings-form-layout.tsx`
- `src/components/error-state.tsx`
- `src/features/developers/page.tsx`, `src/features/developers/request-builder.css`
- `src/features/wallet/index.tsx`, `src/features/wallet/components/native-wallet-checkout.tsx`
- `src/features/auth/auth-layout.tsx`, `src/features/auth/auth-mobile.css`
- `src/features/pricing/index.tsx`, `src/features/pricing/pricing-editorial.css`

## Known Gaps & Exceptions

- [inferred confidence=medium] The source-level observations in this guide do not record a visual acceptance result. Runtime contrast, touch targets, text expansion, focus restoration, reduced motion, and the relevant reflow/zoom scenarios require rendered verification for each changed surface.
- [inferred confidence=medium] A feature may depart from console density only when its own confirmed product declaration or first-party reference explicitly introduces a different visual world.

### Language Model Mesh logo

- [observed] The default brand is LMM API (Language Model Mesh, 语言模型网络). Its folded ribbon symbol and geometric lettering are editable SVG adaptations of the user-supplied master in `.github/assets/lmm-api-source.png`; shared paths live in `.github/assets/logo-geometry.json`.
- [observed] React components inherit `currentColor`, with neutral opacity/gradients for the folds. No external image or font request is needed for built-in branding. Public headers, sidebars and footers retain their existing compact-symbol or wordmark layout.
- [observed] Built-in cached names and logo URLs normalize to LMM API. Tenant-defined names and logos remain configured. Favicon and touch icon derive from the same symbol on a light square.

### Shape, navigation and surfaces

- [observed] The default preset uses a soft `--radius` of 0.75rem. The shared `--cut-size` token remains available to brand geometry; console page titles and homepage section headings use plain type without a prefixed cut mark. Named presets and the user's radius axis continue to override the default.
- [observed] The console sidebar uses a theme-derived tonal surface in both modes. The selected destination uses the secondary container/foreground pair and rounded corners, with an unclipped keyboard focus ring.
- [observed] Homepage display headings use weight 550 and console titles use 600, with tight tracking. The homepage cinema uses a light primary action against its charcoal ground, a muted tonal account action, and visible keyboard focus rings; warm coral remains in the particle artwork.
- [observed] `--font-sans` names the bundled family `'Public Sans Variable'` first, followed by a deterministic CJK sans stack. The user's font axis selects the body face through `--font-body`.
- [observed] Console cards and the sidebar shell use flat tonal surfaces without decorative borders or shadows. Passive cards stay still on hover; selection and focus remain attached to the relevant controls.

### Public homepage pointillist poster exception

- [confirmed] The public homepage uses the user's [Oriku video reference](https://x.com/Oriku175/status/2065701075202806030/video/1) as a code-led poster direction. Its runtime marker is `data-poster-direction="oriku-pointillist-sculpture"`; no approved composition or randomly selected concept is claimed.
- [observed] The homepage leads with the tool market inside an AI community hub, not with API relay terminology. Its primary action opens the tool market; the account-aware next step and pricing remain secondary actions. The cinema uses a deep-charcoal ground (`--forge-night`) in both color modes, warm-coral particles, restrained branding and a large left-aligned display heading. The public header and directory entry overlay the scene; the bottom chapter controls use text labels and tonal selection, putting the tool market before the store. The following connection, assistant and exploration sections use the active public theme, broad gaps and open layouts. The cinema palette remains a homepage exception.
- [observed] Intro → store → tool market → Pi/dsh/directory ecosystem → future makes four native-scroll transitions. Pointer strokes advect nearby points with vertical scatter; time-based critical damping restores their grid after release. Image samples morph into shop, socket and planet sculptures.
- [observed] The renderer budgets 60fps desktop/30fps mobile and uses cached typed buffers. Mobile, short viewports and reduced motion retain five readable chapters; static imagery, passive pointer/scroll listeners, offscreen/hidden pausing and owned cleanup support fallback behavior. Lotus provenance and the font's SIL OFL remain beside their assets.

### HTML-in-Canvas and the quiet interface

The interface uses spacing, typography and semantic accents to make the next action clear. Default light surfaces are cool white; dark surfaces are charcoal with lighter accents. Named themes, user-selected radius, density and typography continue to override defaults. Operational state still uses the existing success, warning and destructive roles.

The shared primitives use restrained corners and consistent spacing. Dialogs have room for their title, explanation and actions; empty states keep their named recovery action. Tables and passive cards stay still during reading. Button and row feedback belongs to the interacted control, so changing a filter no longer starts a staggered entrance for every result.

The public homepage adds an interactive connection diagram between the existing cinema and connection instructions. Its six labelled links and central brand remain HTML. In supporting browsers, `content="drawable"`, `drawable` and `drawElementImage()` render the scene through the native 2D canvas pipeline. Capability detection selects the renderer during component initialization. Unsupported browsers and a failed snapshot keep the same readable links. Tenant logos remain ordinary images outside the snapshot, preserving cross-origin branding. The diagram omits decorative coordinates, orbit rings, numbering and pointer glow.

The scene redraws in the canvas `paint` event; resize uses physical pixel dimensions and avoids resetting an unchanged backing store. Pointer feedback pauses when hidden, outside the viewport, explicitly paused or under reduced motion. The established five homepage chapters and account-aware calls to action retain their existing behavior. Financial forms and authentication continue to use their native controls.

Homepage connection instructions and assistant content use open sections, simple connection-method tabs and a level code panel. The original particle chapters and account-aware destinations remain available.

The login page centers a single form column with a maximum width of 25rem and generous vertical spacing. Branding, language and theme controls remain in the header; tonal inputs and the contrasting main action have a `3.25rem` minimum height (52px at the default root font size). The page scrolls naturally on short screens, and optional games mount only when the footer disclosure opens on either desktop or mobile. Password recovery sits beside its field, registration below the form, and account help uses subdued text. Configured legal consent retains its translated links and explicit checkbox; the duplicate sign-in footer is omitted. Authentication, consent and recovery actions retain their existing paths.

Public pricing gives the title, model count and funding action room above a sticky search and filter toolbar. Model tables use tabular numerals, taller rows and quiet separators; filter selection uses semantic tonal fills. The public access explanation uses open columns, a muted journey section and a readable question-and-answer area. The card view uses at most three model columns, with quiet resting filters and clear selected states. Model data, price units, cache quotes, visibility rules, comparison actions and payment links retain their existing sources and behavior. The console greeting is unboxed and keeps account information close to the page title.

Implementation: `src/features/home/html-canvas-mesh.tsx`, `homepage-canvas.css`, the shared primitives and semantic theme files. The API choice follows [Chrome's stable HTML-in-Canvas announcement](https://developer.chrome.com/blog/html-in-canvas), published 2026-10-26. Native rendering and ordinary DOM fallback are separate verification targets; source support alone is not a browser result.

### Material accent roles

The default palette uses blue for primary actions, green for success, amber for warnings, purple for privileged roles, and red for destructive states. Roles retain text/icons; color alone never communicates permissions or money state. Named presets, font, radius, scale and layout remain user choices. User records separate identity, account state and exact wallet units; overview metric icons use primary/info/tertiary tonal containers. Sidebar selection follows the chosen palette instead of forcing a fixed charcoal/coral theme. Reference: https://m3.material.io/styles/color/the-color-system/ .
