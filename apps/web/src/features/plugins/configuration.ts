/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

import { createElement, type ComponentType } from 'react'

import { lazyNamedSection } from '@/features/system-settings/utils/lazy-section'

// Bind each editor to a module-owned write contract. Infrastructure fields
// below are public TOML names used only to explain incomplete host setup.
const editors = new Map<string, ComponentType>([
  [
    'assistant',
    lazyNamedSection(
      () =>
        import('@/features/system-settings/content/conversation-starter-settings'),
      'ConversationStarterSettings'
    ),
  ],
  [
    'community',
    lazyNamedSection(
      () => import('./community-configuration'),
      'CommunityConfiguration'
    ),
  ],
  [
    'checkins',
    lazyNamedSection(
      () =>
        import('@/features/system-settings/general/checkin-settings-section'),
      'CheckinSettingsSection'
    ),
  ],
  [
    'operations',
    lazyNamedSection(
      () => import('@/features/scripts/scripts-panel'),
      'ScriptsSettingsSection'
    ),
  ],
  [
    'payments',
    lazyNamedSection(
      () => import('./payment-configuration'),
      'PaymentConfiguration'
    ),
  ],
  [
    'store',
    lazyNamedSection(
      () => import('./store-configuration'),
      'StoreConfiguration'
    ),
  ],
  [
    'toolmarket',
    lazyNamedSection(
      () => import('@/features/market-ai-review/settings'),
      'MarketAIReviewSettingsSection'
    ),
  ],
])

const sectionEditors = new Map<string, ComponentType>([
  [
    'operations/model-deployment',
    lazyNamedSection(
      () =>
        import('@/features/system-settings/integrations/ionet-deployment-settings-section'),
      'IoNetDeploymentSettingsSection'
    ),
  ],
  [
    'community/links',
    lazyNamedSection(
      () => import('@/features/system-settings/site/ai-directory-section'),
      'AIDirectorySection'
    ),
  ],
])

export const pluginNavigation: Record<
  string,
  ReadonlyArray<{ id?: string; label: string }>
> = {
  operations: [
    { label: 'Scripts' },
    { id: 'model-deployment', label: 'io.net Deployments' },
  ],
  community: [
    { label: 'Header navigation' },
    { id: 'links', label: 'AI directory' },
  ],
}

export function pluginEditor(name: string, section?: string) {
  return section ? sectionEditors.get(`${name}/${section}`) : editors.get(name)
}

export function renderPluginEditor(name: string, section?: string) {
  const Editor = pluginEditor(name, section)
  return Editor ? createElement(Editor) : null
}

const fields: Record<string, readonly string[]> = {
  acquisition: ['host.public_origin', 'acquisition.database_url_file'],
  assistant: ['core.assistant_token_file', 'assistant.database_url_file'],
  checkins: ['core.checkin_token_file', 'checkins.database_url_file'],
  'hero-sms': ['hero_sms.core_socket', 'hero_sms.core_token_file'],
  operations: ['operations.database_url_file'],
  payments: [
    'core.payment_token_file',
    'payments.database_url_file',
    'payments.config_file',
  ],
  store: ['store.database_url_file'],
}

export function pluginHostFields(name: string) {
  return Object.hasOwn(fields, name) ? fields[name] : []
}
