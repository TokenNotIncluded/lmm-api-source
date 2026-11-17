/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { TFunction } from 'i18next'

export function pluginPresentation(name: string, t: TFunction) {
  switch (name) {
    case 'identity':
      return {
        title: t('Identity'),
        description: t('Account connections and identity features.'),
      }
    case 'assistant':
      return {
        title: t('Assistant'),
        description: t('Assistant conversations and connected tools.'),
      }
    case 'payments':
      return {
        title: t('Payments'),
        description: t('Payment providers and order processing.'),
      }
    case 'operations':
      return {
        title: t('Operations'),
        description: t('Operational records and administrative tools.'),
      }
    case 'store':
      return {
        title: t('Store'),
        description: t('Products, purchases and delivery records.'),
      }
    case 'support':
      return {
        title: t('Support'),
        description: t('Support requests and customer conversations.'),
      }
    case 'community':
      return {
        title: t('Community'),
        description: t('Community posts and shared content.'),
      }
    case 'checkins':
      return {
        title: t('Check-ins'),
        description: t('Daily check-ins and configured rewards.'),
      }
    case 'acquisition':
      return {
        title: t('Acquisition'),
        description: t('Invitations and acquisition campaigns.'),
      }
    case 'hero-sms':
      return {
        title: 'Hero SMS',
        description: t('SMS activation services and their records.'),
      }
    case 'remote-control':
      return {
        title: t('Remote control'),
        description: t('Remote connections and control sessions.'),
      }
    case 'toolmarket':
      return {
        title: t('Tool market'),
        description: t('Published tools, connections and invocations.'),
      }
    default:
      return { title: name, description: '' }
  }
}
