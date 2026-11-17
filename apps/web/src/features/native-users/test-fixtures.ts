/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  NATIVE_USERS_CAPABILITIES,
  type NativeUser,
  type NativeUserUsage,
} from './contract'

export function nativeUserFixture(
  overrides: Partial<NativeUser> = {}
): NativeUser {
  return {
    id: '9007199254740993',
    personal_account_id: '9007199254740995',
    auth_version: '9007199254740997',
    teams: [
      {
        id: '12',
        account_id: '24',
        name: 'Synthetic research team',
        role: 'member',
        can_spend: true,
      },
      {
        id: '13',
        account_id: '25',
        name: 'Synthetic design team',
        role: 'admin',
        can_spend: false,
      },
    ],
    profile_available: true,
    username: 'synthetic-native-user',
    display_name: '',
    email: 'synthetic-native@example.invalid',
    platform_role: 'user',
    role: 1,
    active: true,
    status: 1,
    account_active: true,
    groups: ['alpha', 'team-beta'],
    core_account_created_at_unix_ms: 0,
    registered_at_unix_ms: -1,
    wallet: {
      status: 'available',
      unit: 'core_credit',
      available_units: '-7',
      wallet_units: '0',
      reserved_units: '3',
      debt_units: '7',
      money_basis: 'unknown',
      money_available: false,
      currency: null,
      provenance: {
        reader: 'financial_profile.read_wallet.v1',
        user_id: '9007199254740993',
        personal_account_id: '9007199254740995',
      },
    },
    ...overrides,
  }
}

export function nativeListFixture(
  items = [nativeUserFixture()],
  page = 1,
  total = items.length
) {
  return {
    version: 1,
    items,
    page,
    page_size: 20,
    total,
    capabilities: structuredClone(NATIVE_USERS_CAPABILITIES),
  }
}

export const nativeStatusFixture = {
  success: true,
  data: {
    admin_users_protocol: 'core.admin-users.v1',
    capabilities: { admin_users_read: true },
  },
}

export function nativeUsageFixture(
  overrides: Partial<Extract<NativeUserUsage, { status: 'available' }>> = {}
): Extract<NativeUserUsage, { status: 'available' }> {
  return {
    status: 'available',
    unit: 'core_credit',
    scope: 'actor_personal_usage',
    historical_imports_included: false,
    settled_requests: '12',
    pending_requests: '2',
    gross_settled_units: '1200',
    refunded_units: '200',
    net_consumption_units: '1000',
    personal_wallet_consumption_units: '700',
    team_wallet_consumption_units: '200',
    subscription_consumption_units: '80',
    sponsored_consumption_units: '20',
    ...overrides,
  }
}
