/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  buildRequestRuleExpr,
  evaluateBillingExpression,
  requestRuleGroupsFromTrace,
  splitBillingExprAndRequestRules,
  tryParseRequestRuleExpr,
  type TimeCondition,
} from './billing-expr'

const range = (
  rangeStart: string,
  rangeEnd: string,
  timeFunc: TimeCondition['timeFunc'] = 'hour'
): TimeCondition => ({
  source: 'time',
  timeFunc,
  timezone: 'UTC',
  mode: 'range',
  value: '',
  rangeStart,
  rangeEnd,
})
const build = (condition: TimeCondition) =>
  buildRequestRuleExpr([{ conditions: [condition], multiplier: '2' }])
const valueAt = (expression: string, hour: number) =>
  evaluateBillingExpression(
    expression.replaceAll('hour("UTC")', String(hour)),
    {}
  ).value

test('09–12 is a half-open same-day range, not an all-day match', () => {
  const expression = build(range('09', '12'))
  assert.equal(expression, '(hour("UTC") >= 9 && hour("UTC") < 12 ? 2 : 1)')
  for (let hour = 0; hour < 24; hour++) {
    assert.equal(valueAt(expression, hour), hour >= 9 && hour < 12 ? 2 : 1)
  }
})

test('21–06 wraps midnight and preserves the existing overnight expression', () => {
  const expression = build(range('21', '6'))
  assert.equal(expression, '(hour("UTC") >= 21 || hour("UTC") < 6 ? 2 : 1)')
  const parsed = tryParseRequestRuleExpr(expression)
  assert.ok(parsed)
  assert.equal(buildRequestRuleExpr(parsed), expression)
  for (let hour = 0; hour < 24; hour++) {
    assert.equal(valueAt(expression, hour), hour >= 21 || hour < 6 ? 2 : 1)
  }
})

test('equal, malformed, fractional and out-of-domain endpoints throw instead of dropping the condition', () => {
  for (const [start, end] of [
    ['9', '9'],
    ['09', '9'],
    ['', '12'],
    ['x', '12'],
    ['9', 'NaN'],
    ['9', 'Infinity'],
    ['9', '1e309'],
    ['9.5', '12'],
    ['-1', '12'],
    ['9', '24'],
    ['25', '6'],
  ]) {
    assert.throws(() => build(range(start, end)), /invalid hour range/)
  }
  for (const [func, min, max] of [
    ['minute', 0, 59],
    ['weekday', 0, 6],
    ['month', 1, 12],
    ['day', 1, 31],
  ] as const) {
    assert.doesNotThrow(() => build(range(String(min), String(max), func)))
    assert.throws(
      () => build(range(String(min - 1), String(max), func)),
      /invalid .* range/
    )
    assert.throws(
      () => build(range(String(min), String(max + 1), func)),
      /invalid .* range/
    )
  }
})

test('time ranges retain grouping with another condition and another multiplier group', () => {
  for (const [start, end] of [
    ['9', '12'],
    ['21', '6'],
  ]) {
    const expression = buildRequestRuleExpr([
      {
        conditions: [
          range(start, end),
          { source: 'header', path: 'x-rate', mode: 'eq', value: 'special' },
        ],
        multiplier: '2',
      },
      { conditions: [range('10', '11')], multiplier: '3' },
    ])
    for (const special of [false, true]) {
      for (let hour = 0; hour < 24; hour++) {
        const inRange =
          Number(start) < Number(end)
            ? hour >= Number(start) && hour < Number(end)
            : hour >= Number(start) || hour < Number(end)
        assert.equal(
          valueAt(
            expression.replaceAll(
              'header("x-rate")',
              JSON.stringify(special ? 'special' : 'ordinary')
            ),
            hour
          ),
          (inRange && special ? 2 : 1) * (hour === 10 ? 3 : 1)
        )
      }
    }
  }
  assert.throws(
    () =>
      buildRequestRuleExpr([
        {
          conditions: [
            range('', '12'),
            { source: 'header', path: 'x-rate', mode: 'eq', value: 'special' },
          ],
          multiplier: '2',
        },
      ]),
    /invalid hour range/
  )
})

test('stored time expressions keep their literal meaning when the visual control cannot represent them', () => {
  const daytimeOr = 'hour("UTC") >= 9 || hour("UTC") < 12'
  for (let hour = 0; hour < 24; hour++) {
    assert.equal(valueAt(`(${daytimeOr} ? 2 : 1)`, hour), 2)
  }
  const custom = [
    daytimeOr,
    `(${daytimeOr}) && header("x-rate") == "special"`,
    'hour("UTC") >= 21 || hour("UTC") < 6 && header("x-rate") == "special"',
    'hour("UTC") >= 9 || hour("UTC") < 9',
    'hour("UTC") >= 24 || hour("UTC") < 6',
    'hour("UTC") >= 9.5 || hour("UTC") < 6',
  ]
  for (const condition of custom) {
    const rule = `(${condition} ? 2 : 1)`
    assert.equal(tryParseRequestRuleExpr(rule), null, condition)
    assert.deepEqual(tryParseRequestRuleExpr(rule, 'display'), [
      { conditions: [], conditionText: condition, multiplier: '2' },
    ])
    assert.deepEqual(splitBillingExprAndRequestRules(`v1:(p) * ${rule}`), {
      billingExpr: 'v1:p',
      requestRuleExpr: rule,
    })
    const trace = requestRuleGroupsFromTrace([
      { cond: condition, matched: true, multiplier: 2 },
    ])[0]
    assert.deepEqual(trace.conditions, [])
    assert.equal(trace.conditionText, condition)
  }
})

test('a grouped overnight condition remains editable with an additional condition', () => {
  const group =
    '(hour("UTC") >= 21 || hour("UTC") < 6) && header("x-rate") == "special"'
  const rule = `(${group} ? 2 : 1)`
  const parsed = tryParseRequestRuleExpr(rule)
  assert.ok(parsed)
  assert.equal(buildRequestRuleExpr(parsed), rule)
})
