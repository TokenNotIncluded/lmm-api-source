/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { l0PreviewUser } from '@/stores/auth-store'

/** Synthetic empty account facts; no administrator response is reused. */
export function l0PreviewRead(path: string, search: URLSearchParams): unknown {
  switch (path) {
    case '/api/remote-control/v1/pi/sessions':
      return { sessions: [] }
    case '/api/user/account-links':
      return {
        contract: 'core.account-links.v1',
        expected_auth_version: '1',
        links: [],
        providers: Object.entries({
          google: 'https://accounts.google.com',
          github: 'https://github.com',
          linuxdo: 'https://connect.linux.do',
          discord: 'https://discord.com',
        }).map(([provider, issuer]) => ({ provider, issuer, can_bind: false })),
      }
    case '/api/user/2fa/status':
      return { enabled: false, locked: false, backup_codes_remaining: 0 }
    case '/api/user/passkey':
      return { enabled: false, credentials: [] }
    case '/api/user/gift':
      return []
    case '/api/user/self/closure':
      return {
        contract: 'core.account-closure.v1',
        expected_auth_version: '1',
        username: 'L0',
        allowed: false,
        blockers: ['financial_history_unavailable'],
      }
    case '/api/user/self':
      return l0PreviewUser
    case '/api/user/self/announcements':
    case '/api/data/self':
    case '/api/user/models':
    case '/api/ratio-notifications':
    case '/api/user/sessions':
    case '/api/user/auth/oauth2/grants':
      return []
    case '/api/release-notes/latest':
    case '/api/assistant/new-user-gift':
      return null
    case '/api/assistant/support/eligibility':
      return { eligible: false }
    case '/api/assistant/support/self':
      return { request: null }
    case '/api/assistant/registration-check':
      return { state: 'context_needed' }
    case '/api/user/self/profile-share':
      return {
        enabled: false,
        model_usage_enabled: false,
        aggregate_usage_enabled: false,
        linked_profiles: [],
        aggregate_sources: [],
      }
    case '/api/assistant/pre-conversation-presets':
      return { generation: 0, version: 'preview', presets: [] }
    case '/api/assistant/journey':
      return { main: [], side: [] }
    case '/api/assistant/status':
      return {
        model: '',
        funding: { mode: 'super_administrator' },
        enabled: true,
        developer_access_granted: false,
        trust_level: 0,
        role: 1,
        is_admin: false,
        is_root: false,
        capabilities: {
          public_assistant: true,
          account: true,
          developer_tools: false,
          admin_config: false,
        },
      }
    case '/api/assistant/conversations':
      return { conversations: [], privacy_notice: '' }
    case '/api/todos':
      return {
        items: [],
        page: Number(search.get('p') || 1),
        page_size: 50,
        total: 0,
        category: search.get('category') || 'all',
        unread_count: 0,
        total_unread_count: 0,
        unread_by_category: {},
        categories: [],
      }
    default:
      return undefined
  }
}

export function isL0PreviewPublicRead(path: string): boolean {
  return (
    [
      '/api/status',
      '/api/notice',
      '/api/pricing',
      '/api/tool-market',
      '/api/tool-market/config',
      '/api/bounties',
      '/api/bounties/config',
      '/api/store/config',
      '/api/store/disclaimer',
      '/api/store/products',
      '/api/store/categories',
      '/api/scripts',
    ].includes(path) ||
    /^\/api\/tool-market\/services\/[^/]+$/.test(path) ||
    /^\/api\/bounties\/projects\/[^/]+$/.test(path) ||
    /^\/api\/store\/products\/[^/]+(?:\/(?:terms|likes))?$/.test(path)
  )
}
