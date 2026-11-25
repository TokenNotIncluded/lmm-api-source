/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeChannel } from './native-checkout'

export const paymentProviderNames: Record<NativeChannel['provider'], string> = {
  epay: 'EPay',
  stripe: 'Stripe',
  creem: 'Creem',
  waffo: 'Waffo',
  waffo_pancake: 'Waffo Pancake',
  coinbase: 'Coinbase',
}
