/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import type { PaymentMethodData } from './payment-method-dialog'

export type PaymentMethodTemplate = {
  labelKey: string
  method: PaymentMethodData
}

/**
 * Safe, one-click payment methods. Custom Epay methods deliberately do not
 * appear here: their `type` is forwarded directly to the configured gateway,
 * so the administrator must provide the gateway-supported value explicitly.
 */
export const PAYMENT_METHOD_TEMPLATES: readonly PaymentMethodTemplate[] = [
  {
    labelKey: 'Epay Alipay',
    method: { icon: 'SiAlipay', name: '支付宝', type: 'alipay' },
  },
  {
    labelKey: 'Epay WeChat Pay',
    method: { icon: 'SiWechat', name: '微信', type: 'wxpay' },
  },
  {
    labelKey: 'LINUX DO Credit',
    method: {
      icon: 'SiLinux',
      name: 'LINUX DO Credit',
      settlement_unit: 'LDC',
      type: 'epay',
      unit_price: '10',
    },
  },
  {
    labelKey: 'Stripe',
    method: {
      icon: 'SiStripe',
      min_topup: '10',
      name: 'Stripe',
      type: 'stripe',
    },
  },
  {
    labelKey: 'Creem',
    method: { name: 'Creem', type: 'creem' },
  },
  {
    labelKey: 'Waffo',
    method: { name: 'Waffo', type: 'waffo' },
  },
  {
    labelKey: 'Waffo Pancake',
    // Waffo Pancake has a dedicated wallet SVG. Do not override it here.
    method: { name: 'Waffo Pancake', type: 'waffo_pancake' },
  },
]

export function insertPaymentMethodTemplate(
  methods: readonly unknown[],
  template: PaymentMethodData
): unknown[] {
  const alreadyPresent = methods.some(
    (method) =>
      typeof method === 'object' &&
      method !== null &&
      'name' in method &&
      'type' in method &&
      method.name === template.name &&
      method.type === template.type
  )

  if (alreadyPresent) return [...methods]

  return [...methods, { ...template }]
}
