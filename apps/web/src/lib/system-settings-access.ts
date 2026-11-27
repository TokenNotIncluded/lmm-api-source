/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import {
  canOpenConfiguration,
  configurationOwner,
  legacyPluginPanel,
} from '../features/plugins/configuration'
import { writableOptionKey } from '../features/system-settings/hooks/option-writability'
import type { SystemOptionsResponse } from '../features/system-settings/types'
import { ROLE } from './roles'

export const CORE_SETTINGS_HOME = '/system-settings'
export const EXTENSION_SETTINGS_HOME = '/extensions/plugins'
const LEGACY_EXTENSION_SETTINGS_HOME = '/extensions/settings'
const PLUGINS = EXTENSION_SETTINGS_HOME
export const ASSISTANT_PRESET_SETTINGS_PATH =
  '/system-settings/content/assistant'

export type SettingsScope = 'core' | 'extensions'
type SectionPolicy = {
  owner: 'core' | 'plugin' | 'shared' | 'compatibility' | 'unavailable'
  writableKeys?: readonly string[]
  pluginOwners?: readonly string[]
}

const core: SectionPolicy = { owner: 'core' }
const plugin: SectionPolicy = { owner: 'plugin' }
const sharedReview: SectionPolicy = {
  owner: 'shared',
  pluginOwners: ['store', 'toolmarket'],
}
const unavailable: SectionPolicy = { owner: 'unavailable' }
const coreOptions = (...writableKeys: string[]): SectionPolicy => ({
  owner: 'core',
  writableKeys,
})
const compatibility = (...writableKeys: string[]): SectionPolicy => ({
  owner: 'compatibility',
  writableKeys,
})

// Legacy option names come from the inputs of the registered form. Dedicated
// native routes continue to run their own capability and authorization checks.
const sections: Readonly<Record<string, SectionPolicy>> = {
  'site/system-info': compatibility(
    'SystemName',
    'Logo',
    'Footer',
    'About',
    'HomePageContent',
    'legal.user_agreement',
    'legal.privacy_policy',
    'legal.user_agreement_en',
    'legal.privacy_policy_en',
    'legal.refund_policy',
    'legal.refund_policy_en'
  ),
  'site/notice': coreOptions('Notice'),
  'site/header-navigation': plugin,
  'site/sidebar-modules': compatibility('SidebarModulesAdmin'),
  'site/ai-directory': plugin,
  'site/rss': plugin,
  'site/ai-directory-ads': plugin,
  'auth/humangrant': core,
  'auth/basic-auth': compatibility(
    'PasswordLoginEnabled',
    'PasswordRegisterEnabled',
    'OAuthRegisterEnabled',
    'RegistrationDisabledMethods',
    'EmailVerificationEnabled',
    'RegisterEnabled',
    'EmailDomainRestrictionEnabled',
    'EmailAliasRestrictionEnabled',
    'EmailDomainWhitelist'
  ),
  'auth/oauth': compatibility(
    'GitHubOAuthEnabled',
    'GitHubClientId',
    'GitHubClientSecret',
    'discord.enabled',
    'discord.client_id',
    'discord.client_secret',
    'oidc.enabled',
    'oidc.display_name',
    'oidc.client_id',
    'oidc.client_secret',
    'oidc.well_known',
    'oidc.authorization_endpoint',
    'oidc.token_endpoint',
    'oidc.user_info_endpoint',
    'TelegramOAuthEnabled',
    'TelegramBotToken',
    'TelegramBotName',
    'LinuxDOOAuthEnabled',
    'LinuxDOClientId',
    'LinuxDOClientSecret',
    'LinuxDOMinimumTrustLevel',
    'WeChatAuthEnabled',
    'WeChatServerAddress',
    'WeChatServerToken',
    'WeChatAccountQRCodeImageURL'
  ),
  'auth/passkey': compatibility(
    'passkey.enabled',
    'passkey.rp_display_name',
    'passkey.rp_id',
    'passkey.origins',
    'passkey.allow_insecure_origin',
    'passkey.user_verification',
    'passkey.attachment_preference'
  ),
  'auth/bot-protection': compatibility(
    'TurnstileCheckEnabled',
    'TurnstileSiteKey',
    'TurnstileSecretKey'
  ),
  'auth/custom-oauth': core,
  'security/honeypot': core,
  'security/market-ai-review': sharedReview,
  'security/moderation': compatibility(
    'ModerationEnabled',
    'ModerationPolicyScope',
    'ModerationSafetyIdentifierEnabled',
    'ModerationGroup',
    'ModerationModel',
    'ModerationGroupPolicies'
  ),
  'security/rate-limit': core,
  'security/sensitive-words': compatibility(
    'CheckSensitiveEnabled',
    'CheckSensitiveOnPromptEnabled',
    'SensitiveWords'
  ),
  'security/advanced-security': unavailable,
  'security/ip-access-routing': coreOptions('IPAccessRoutingRules'),
  'security/anti-relay': compatibility(
    'AntiRelayEnabled',
    'AntiRelayRejectProxyHeadersEnabled',
    'AntiRelayHTTPSOnlyEnabled',
    'AntiRelayBlockedCIDRs',
    'AntiRelayTrustedProxyCIDRs'
  ),
  'security/ssrf': compatibility(
    'fetch_setting.enable_ssrf_protection',
    'fetch_setting.allow_private_ip',
    'fetch_setting.domain_filter_mode',
    'fetch_setting.ip_filter_mode',
    'fetch_setting.domain_list',
    'fetch_setting.ip_list',
    'fetch_setting.allowed_ports',
    'fetch_setting.apply_ip_filter_for_domain'
  ),
  'security/token-limits': compatibility('token_setting.max_user_tokens'),
  'models/service-tier-pricing': unavailable,
  'models/global': core,
  'models/routing-reliability': core,
  'models/gemini': compatibility(
    'gemini.safety_settings',
    'gemini.version_settings',
    'gemini.supported_imagine_models',
    'gemini.thinking_adapter_enabled',
    'gemini.thinking_adapter_budget_tokens_percentage',
    'gemini.function_call_thought_signature_enabled',
    'gemini.remove_function_response_id_enabled'
  ),
  'models/claude': compatibility(
    'claude.model_headers_settings',
    'claude.default_max_tokens',
    'claude.thinking_adapter_enabled',
    'claude.refusal_no_output_no_charge_enabled',
    'claude.thinking_adapter_budget_tokens_percentage'
  ),
  'models/grok': compatibility(
    'grok.violation_deduction_enabled',
    'grok.violation_deduction_amount'
  ),
  'models/channel-affinity': core,
  'models/model-deployment': plugin,
  'content/dashboard': compatibility(
    'DataExportEnabled',
    'DataExportInterval',
    'DataExportDefaultTime'
  ),
  'content/announcements': coreOptions(
    'Announcements',
    'console_setting.announcements_enabled',
    'console_setting.announcements'
  ),
  'content/api-info': compatibility(
    'console_setting.api_info_enabled',
    'console_setting.api_info'
  ),
  'content/faq': compatibility(
    'console_setting.faq_enabled',
    'console_setting.faq'
  ),
  'content/uptime-kuma': compatibility(
    'console_setting.uptime_kuma_enabled',
    'console_setting.uptime_kuma_groups'
  ),
  'content/assistant': plugin,
  'content/chat': compatibility('Chats'),
  'content/drawing': compatibility(
    'DrawingEnabled',
    'MjNotifyEnabled',
    'MjAccountFilterEnabled',
    'MjForwardUrlEnabled',
    'MjModeClearEnabled',
    'MjActionCheckSuccessEnabled'
  ),
  'operations/behavior': compatibility(
    'DefaultCollapseSidebar',
    'DemoSiteEnabled',
    'SelfUseModeEnabled'
  ),
  'operations/alerts': compatibility(
    'QuotaRemindThreshold',
    'perf_metrics_setting.enabled',
    'perf_metrics_setting.flush_interval',
    'perf_metrics_setting.bucket_time',
    'perf_metrics_setting.retention_days'
  ),
  'operations/email': compatibility(
    'SMTPServer',
    'SMTPPort',
    'SMTPAccount',
    'SMTPFrom',
    'SMTPToken',
    'SMTPSSLEnabled',
    'SMTPStartTLSEnabled',
    'SMTPInsecureSkipVerify',
    'SMTPForceAuthLogin'
  ),
  'operations/worker': compatibility(
    'WorkerUrl',
    'WorkerValidKey',
    'WorkerAllowHttpImageRequestEnabled'
  ),
  'operations/hero-sms': plugin,
  'operations/logs': compatibility('LogConsumeEnabled'),
  'operations/performance': compatibility(
    'performance_setting.disk_cache_enabled',
    'performance_setting.disk_cache_threshold_mb',
    'performance_setting.disk_cache_max_size_mb',
    'performance_setting.disk_cache_path',
    'performance_setting.monitor_enabled',
    'performance_setting.monitor_cpu_threshold',
    'performance_setting.monitor_memory_threshold',
    'performance_setting.monitor_disk_threshold'
  ),
  'operations/raw-json': compatibility(
    'group_ratio_setting.group_warnings',
    'GroupRatio',
    'GroupGroupRatio',
    'UserUsableGroups',
    'AutoGroups',
    'group_ratio_setting.group_special_usable_group',
    'AssistantSkillFiles',
    'global.thinking_model_blacklist',
    'global.chat_completions_to_responses_policy',
    'gemini.safety_settings',
    'gemini.version_settings',
    'gemini.supported_imagine_models',
    'claude.model_headers_settings',
    'claude.default_max_tokens',
    'billing_setting.billing_mode',
    'billing_setting.billing_expr',
    'tool_price_setting.prices',
    'channel_affinity_setting.rules',
    'console_setting.api_info',
    'console_setting.announcements',
    'console_setting.faq',
    'payment_setting.amount_options',
    'payment_setting.amount_discount',
    'PayMethods',
    'CreemProducts',
    'WaffoPayMethods'
  ),
  'operations/update-checker': compatibility('UpdateSources'),
  'operations/scripts': plugin,
  'billing/quota': core,
  'billing/currency': coreOptions('USDExchangeRate', 'DisplayTokenStatEnabled'),
  'billing/model-pricing': core,
  'billing/router-pricing': core,
  'billing/group-pricing': core,
  'billing/payment': core,
  'billing/checkin': plugin,
}

export type SettingsBoundary = {
  owner: string
  canonical: string
  writableKeys?: readonly string[]
  pluginOwners?: readonly string[]
}

function parts(url: string) {
  const [beforeHash, hash = ''] = url.split('#', 2)
  const [path, query = ''] = beforeHash.split('?', 2)
  return { path: path.replace(/\/+$/, ''), hash, query }
}

export function getSettingsBoundary(url: string): SettingsBoundary | undefined {
  const { path, query, hash } = parts(url)
  const panel =
    path === PLUGINS
      ? (new URLSearchParams(query).get('configure') ?? undefined)
      : legacyPluginPanel(path, hash)
  if (path === PLUGINS && panel === 'payments') {
    return { owner: 'core', canonical: '/system-settings/billing/payment' }
  }
  if (panel) {
    if (panel === 'toolmarket' || panel === 'store-review') {
      return { ...sharedReview, canonical: `${PLUGINS}?configure=${panel}` }
    }
    const owner = configurationOwner(panel)
    return owner
      ? {
          owner,
          canonical: `${PLUGINS}?configure=${encodeURIComponent(panel)}`,
        }
      : undefined
  }
  if (path === PLUGINS || path === LEGACY_EXTENSION_SETTINGS_HOME) {
    return { owner: 'plugins', canonical: PLUGINS }
  }
  if (path === CORE_SETTINGS_HOME || path === '/users') {
    return { owner: 'core', canonical: path }
  }
  if (path === '/keys' || path === '/profile' || path === '/profile/share') {
    return { owner: 'personal', canonical: path }
  }
  if (path === '/store/settings') return { owner: 'merchant', canonical: path }
  if (!path.startsWith(`${CORE_SETTINGS_HOME}/`)) return undefined
  const id = path.slice(CORE_SETTINGS_HOME.length + 1)
  if (!Object.hasOwn(sections, id)) return undefined
  const section = sections[id]
  if (section.owner === 'plugin') return undefined
  return { ...section, canonical: path }
}

export function isSettingsBoundaryVisible(
  url: string,
  options?: SystemOptionsResponse
): boolean {
  const boundary = getSettingsBoundary(url)
  if (!boundary || boundary.owner === 'unavailable') return false
  if (!boundary.writableKeys) return true
  if (options?.success !== true || !Array.isArray(options.data)) return false
  // Generic legacy forms are available only after a successful full option
  // read. Native projections advertise the keys their narrow editors accept.
  if (
    boundary.owner === 'compatibility' &&
    options.capabilities !== undefined
  ) {
    return false
  }
  return boundary.writableKeys.some((key) => {
    const writable = writableOptionKey(options.capabilities, key)
    return (
      writable !== undefined &&
      options.data.some((item) => item.key === writable)
    )
  })
}

const categories = new Set(Object.keys(sections).map((id) => id.split('/')[0]))

export function canReadSystemSettingsPath(
  role: number | undefined,
  url: string
): boolean {
  const { path, query } = parts(url)
  const root = role === ROLE.SUPER_ADMIN
  if (!root && role !== ROLE.ADMIN) return false
  if (
    path === CORE_SETTINGS_HOME ||
    path === ASSISTANT_PRESET_SETTINGS_PATH ||
    path === '/system-settings/security/market-ai-review'
  ) {
    return true
  }
  if (path === PLUGINS || path === LEGACY_EXTENSION_SETTINGS_HOME) {
    const panel = new URLSearchParams(query).get('configure')
    return (
      panel === null ||
      (panel === 'payments' ? root : canOpenConfiguration(panel, root))
    )
  }
  if (!root) return false
  if (
    path.startsWith(`${CORE_SETTINGS_HOME}/`) &&
    categories.has(path.slice(CORE_SETTINGS_HOME.length + 1))
  ) {
    return true
  }
  const boundary = getSettingsBoundary(url)
  return Boolean(
    boundary && boundary.owner !== 'personal' && boundary.owner !== 'merchant'
  )
}

export function getSettingsScope(url: string): SettingsScope | undefined {
  const boundary = getSettingsBoundary(url)
  if (
    !boundary ||
    ['unavailable', 'personal', 'merchant'].includes(boundary.owner)
  ) {
    return undefined
  }
  if (boundary.owner === 'core' || boundary.owner === 'compatibility') {
    return 'core'
  }
  return 'extensions'
}

export function getSettingsHomeForPath(url: string) {
  const { path } = parts(url)
  if (path === '/users' || path === '/keys') return null
  const scope = getSettingsScope(url)
  return scope === 'extensions'
    ? EXTENSION_SETTINGS_HOME
    : scope === 'core'
      ? CORE_SETTINGS_HOME
      : null
}

export function getSettingsTitle(scope: SettingsScope) {
  return scope === 'core' ? 'Core Settings' : 'Plugin Settings'
}
