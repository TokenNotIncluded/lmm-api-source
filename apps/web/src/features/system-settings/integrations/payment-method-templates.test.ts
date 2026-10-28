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
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  insertPaymentMethodTemplate,
  PAYMENT_METHOD_TEMPLATES,
} from './payment-method-templates'

describe('payment method templates', () => {
  test('defines only gateway-safe built-in payment types', () => {
    assert.deepEqual(
      PAYMENT_METHOD_TEMPLATES.map(({ method }) => method.type),
      ['alipay', 'wxpay', 'epay', 'stripe', 'creem', 'waffo', 'waffo_pancake']
    )
    assert.equal(
      PAYMENT_METHOD_TEMPLATES.find(
        ({ method }) => method.type === 'waffo_pancake'
      )?.method.icon,
      undefined
    )
  })

  test('inserts each built-in template once by name and type', () => {
    const linuxDoTemplate = PAYMENT_METHOD_TEMPLATES.find(
      ({ method }) => method.type === 'epay'
    )
    assert.ok(linuxDoTemplate)
    const linuxDo = linuxDoTemplate.method
    const once = insertPaymentMethodTemplate([], linuxDo)
    const twice = insertPaymentMethodTemplate(once, linuxDo)

    assert.notStrictEqual(once[0], linuxDo)
    assert.deepEqual(twice, once)
    assert.notStrictEqual(twice, once)
    assert.notStrictEqual(twice[0], linuxDo)
  })

  test('does not treat a different display name as the same template', () => {
    const alipay = PAYMENT_METHOD_TEMPLATES[0].method
    const methods = insertPaymentMethodTemplate(
      [{ ...alipay, name: '支付宝（备用）' }],
      alipay
    )

    assert.equal(methods.length, 2)
  })

  test('preserves unresolved raw JSON entries while inserting', () => {
    const rawEntries: unknown[] = [null, 'legacy-entry', { future: true }]
    const methods = insertPaymentMethodTemplate(
      rawEntries,
      PAYMENT_METHOD_TEMPLATES[0].method
    )

    assert.deepEqual(methods.slice(0, rawEntries.length), rawEntries)
    assert.equal(methods.length, rawEntries.length + 1)
  })
})
