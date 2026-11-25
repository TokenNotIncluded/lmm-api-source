/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { AuthSectionId } from './auth/section-registry'
import type { BillingSectionId } from './billing/section-registry'
import type { ContentSectionId } from './content/section-registry'
import type { ModelSectionId } from './models/section-registry'
import type { OperationsSectionId } from './operations/section-registry'
import type { SecuritySectionId } from './security/section-registry'
import type { SiteSectionId } from './site/section-registry'

type Sections = {
  auth: AuthSectionId
  billing: BillingSectionId
  content: ContentSectionId
  models: ModelSectionId
  operations: OperationsSectionId
  security: SecuritySectionId
  site: SiteSectionId
}

const identity = '/system-settings/site?domain=identity'
const billing = '/system-settings/site?domain=billing'
const routing = '/system-settings/site?domain=routing'
const groupPrices = '/channels?view=group-pricing'
const modelPrices = '/channels?view=model-pricing'
const upstreamCosts = '/channels?view=model-pricing&mode=upstream_cost'
const plugin = '/extensions/plugins?configure='

export type LegacySettingsDestination =
  | {
      action: 'redirect'
      coverage: 'supported' | 'partial' | 'read-only'
      href: string
      detail: string
    }
  | {
      action: 'choose'
      coverage: 'partial'
      choices: ReadonlyArray<{ href: string; label: string }>
      detail: string
    }
  | {
      action: 'unavailable'
      coverage: 'pending' | 'unsupported'
      detail: string
    }

function destination(
  href: string,
  detail: string,
  coverage: 'supported' | 'partial' | 'read-only' = 'supported'
): LegacySettingsDestination {
  return { action: 'redirect', href, detail, coverage }
}

function missing(
  detail: string,
  coverage: 'pending' | 'unsupported' = 'unsupported'
): LegacySettingsDestination {
  return { action: 'unavailable', coverage, detail }
}

// An existing consumer page, a different module's announcement or a disabled
// generic option form does not implement the bookmarked administrative control.
const sections = {
  auth: {
    humangrant: destination(
      identity,
      'The host HumanGrant switch is file-backed. The site approval switch has its own authenticated settings contract.',
      'partial'
    ),
    'basic-auth': destination(
      identity,
      'Registration methods, email verification and domain restrictions are Core identity file settings.',
      'partial'
    ),
    oauth: destination(
      identity,
      'Core loads configured login providers and file-backed credentials. Provider-specific options depend on the provider.',
      'partial'
    ),
    passkey: destination(
      identity,
      'Core WebAuthn derives its origin from identity.web and accepts passkey_rp_id. The legacy attachment and verification preferences have no corresponding editor.',
      'partial'
    ),
    'bot-protection': destination(
      identity,
      'Turnstile uses the identity.turnstile site key and a separate secret file.'
    ),
    'custom-oauth': destination(
      identity,
      'Federated providers use the Core provider operation file and revision checks.',
      'partial'
    ),
  },
  billing: {
    quota: {
      action: 'choose',
      coverage: 'partial',
      detail:
        'Automatic registration gifts, invitation percentages, thresholds and penalties have no equivalent online policy.',
      choices: [
        { href: '/redemption-codes', label: 'Promotion campaigns' },
        { href: '/operations/sources', label: 'Promotion links' },
        { href: `${plugin}checkins`, label: 'Check-in rewards' },
      ],
    },
    currency: missing(
      'The display exchange rate is a distinct Core option (USDExchangeRate). A dedicated display-rate control must be attached.',
      'pending'
    ),
    'model-pricing': destination(
      modelPrices,
      'Published model and group-specific tariff rules; unsupported legacy expressions require explicit conversion.',
      'partial'
    ),
    'router-pricing': destination(
      upstreamCosts,
      'Open the upstream cost mode of the pricing editor.'
    ),
    'group-pricing': destination(
      groupPrices,
      'Business group price rules and group-specific pricing belong in channel management.',
      'partial'
    ),
    payment: destination(
      `${plugin}payments`,
      'Wallet channel availability and the Pancake editor. Core provider policies are file-backed; EPay has a separate connection contract.',
      'partial'
    ),
    checkin: destination(
      `${plugin}checkins`,
      'Daily reward enablement and minimum/maximum reward amounts.'
    ),
  },
  content: {
    dashboard: missing(
      'Dashboard export schedule, interval and default period have no native settings contract.'
    ),
    announcements: missing(
      'Core supports console announcements with expected-value validation. The editor must receive the actual stored announcements.',
      'pending'
    ),
    'api-info': missing(
      'The console API address list has no native settings contract.'
    ),
    faq: missing('The console FAQ list has no native settings contract.'),
    'uptime-kuma': missing(
      'Uptime Kuma group configuration has no native settings contract.'
    ),
    assistant: destination(
      `${plugin}assistant`,
      'Conversation starters are editable. The legacy assistant model, moderation, persona, tool and retention policies are separate.',
      'partial'
    ),
    chat: missing(
      'The legacy list of external chat presets has no native settings contract.'
    ),
    drawing: missing(
      'The legacy drawing switches and Midjourney forwarding policy have no corresponding tool-market editor.'
    ),
  },
  models: {
    'service-tier-pricing': missing(
      'Fast and Ultrafast service tier synchronization has no native management endpoint. Token-count tariff tiers are a different pricing feature.'
    ),
    global: missing(
      'Global passthrough, thinking blacklists, Chat-to-Responses conversion and keepalive options have no equivalent native settings contract.'
    ),
    'routing-reliability': destination(
      routing,
      'Core routes have a maximum attempt count and retry causes. Automatic test schedules and legacy disable/status-code rules have no equivalent controls.',
      'partial'
    ),
    gemini: missing(
      'The legacy Gemini safety/version and thinking adapter policy has no native management contract.'
    ),
    claude: missing(
      'The legacy Claude header, default token, refusal and thinking adapter policy has no native management contract.'
    ),
    grok: missing(
      'The legacy Grok violation deduction policy has no native management contract.'
    ),
    'channel-affinity': missing(
      'The legacy channel affinity rules, cache lifetime and switch-on-success policy have no native management contract.'
    ),
    'model-deployment': destination(
      '/extensions/plugins?configure=operations&section=model-deployment',
      'The operations io.net editor reads and updates provider settings and tests the connection.'
    ),
  },
  operations: {
    behavior: missing(
      'The legacy sidebar default, demo site and self-use switches have no native settings contract.'
    ),
    alerts: missing(
      'The legacy low-quota threshold, metrics flush interval and retention policy have no equivalent administrative destination.'
    ),
    email: destination(
      identity,
      'Core identity email delivery uses its own SMTP configuration and credential files. It does not expose a global SMTP options editor.',
      'partial'
    ),
    worker: missing(
      'The legacy image fetch worker URL and key have no native settings contract.'
    ),
    'hero-sms': missing(
      'HeroSMS has authenticated settings and Core policy-file support. A destination for its own policy must be attached.',
      'pending'
    ),
    logs: missing(
      'The legacy LogConsumeEnabled switch and log cleanup control have no corresponding settings endpoint.'
    ),
    performance: missing(
      'Viewing system resources does not configure the old disk-cache limits, monitoring thresholds or retention policy.'
    ),
    'raw-json': {
      action: 'choose',
      coverage: 'partial',
      detail:
        'Choose the policy to change. The old JSON editor also mixed unsupported provider, assistant and console keys.',
      choices: [
        { href: groupPrices, label: 'Group pricing' },
        { href: modelPrices, label: 'Model pricing' },
        { href: upstreamCosts, label: 'Upstream costs' },
        { href: billing, label: 'Core payment configuration' },
      ],
    },
    'update-checker': destination(
      '/system-info',
      'Instance heartbeats expose the running version and start time. The old check-for-updates and automatic maintenance controls are unavailable.',
      'read-only'
    ),
    scripts: destination(
      `${plugin}operations`,
      'The operations scripts editor manages the script repository.'
    ),
  },
  security: {
    honeypot: missing(
      'Core exposes the dedicated honeypot policy, users, previews and audit APIs. The existing HoneypotPage needs an administrative destination.',
      'pending'
    ),
    'market-ai-review': destination(
      `${plugin}toolmarket`,
      'The shared store/tool-market first-review policy has its own settings API.'
    ),
    moderation: missing(
      'Legacy account-group content screening and category deductions have no equivalent native settings API.'
    ),
    'rate-limit': destination(
      routing,
      'Core request limits have a versioned policy for windows and per-user/group limits. Legacy success-only counters are separate.',
      'partial'
    ),
    'sensitive-words': missing(
      'The legacy prompt scanning and sensitive word list have no native management contract.'
    ),
    'advanced-security': missing(
      'The old moderation review, deduction and appeal dashboard requires legacy business endpoints. Honeypot audit is a separate data set.'
    ),
    'ip-access-routing': destination(
      identity,
      'Core enforces the file-backed IPAccessRoutingRules for the edge and direct network access checks.'
    ),
    'anti-relay': missing(
      'The old proxy-header rules, CIDR lists and HTTPS-only options have no equivalent settings contract.'
    ),
    ssrf: missing(
      'The old global fetch allow/deny lists and private IP switches have no equivalent administrative control.'
    ),
    'token-limits': missing(
      'The site-wide maximum count of user tokens has no equivalent administrative control.'
    ),
  },
  site: {
    'system-info': missing(
      'Core projects ServerAddress and has separate deployment legal files. The legacy branding, content and policy editor has no matching administrative destination.',
      'pending'
    ),
    notice: missing(
      'Core supports the site Notice option. Its editor needs the current stored value and write authorization.',
      'pending'
    ),
    'header-navigation': destination(
      `${plugin}community`,
      'The community header-navigation editor manages the public navigation.'
    ),
    'sidebar-modules': missing(
      'SidebarModulesAdmin has no native administrative contract.'
    ),
    'ai-directory': destination(
      '/extensions/plugins?configure=community&section=links',
      'The curated links editor reads the community directory and saves through its administrator endpoint.'
    ),
    rss: destination(
      '/rss?manage=feeds',
      'The operations RSS editor sets feed names, source URLs, order and enabled status.'
    ),
    'ai-directory-ads': destination(
      '/ai-directory?manage=ads',
      'The community directory-ad administration supports reviewing and hiding paid placements.'
    ),
  },
} as const satisfies {
  [K in keyof Sections]: Record<Sections[K], LegacySettingsDestination>
}

/** Unknown bookmarks return undefined; empty, foreign or inherited keys never resolve. */
export function legacySettingsDestination(
  category: string,
  section: string
): LegacySettingsDestination | undefined {
  if (!Object.hasOwn(sections, category)) return undefined
  const entries: Readonly<Record<string, LegacySettingsDestination>> =
    sections[category as keyof Sections]
  return Object.hasOwn(entries, section) ? entries[section] : undefined
}
