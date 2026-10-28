/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import {
  ENDPOINT_TYPES,
  FILTER_ALL,
  QUOTA_TYPES,
  SORT_OPTIONS,
} from '../constants'
import type { PricingGroupPrices, PricingModel } from '../types'
import { filterAndSortModels, sortModels } from './filters'

function model(
  id: number,
  modelName: string,
  vendorName: string,
  enableGroups: string[]
): PricingModel {
  return {
    id,
    model_name: modelName,
    vendor_name: vendorName,
    quota_type: 0,
    pricing_schema_version: 2,
    pricing_currency: 'USD',
    model_ratio: 1,
    completion_ratio: 1,
    enable_groups: enableGroups,
  }
}

const noOtherFilters = {
  search: '',
  quotaType: QUOTA_TYPES.ALL,
  endpointType: ENDPOINT_TYPES.ALL,
  tag: FILTER_ALL,
  sortBy: SORT_OPTIONS.NAME,
}

describe('pricing model filters', () => {
  test('price sorting leaves dynamic expressions after scalar prices without using stale legacy fields', () => {
    const cheap = {
      ...model(1, 'cheap', 'Alpha', ['all']),
      quota_type: 1,
      model_price: 1,
    }
    const expensive = {
      ...model(2, 'expensive', 'Alpha', ['all']),
      quota_type: 1,
      model_price: 10,
    }
    const fixed = {
      ...model(3, 'fixed', 'Alpha', ['all']),
      billing_mode: 'tiered_expr',
      billing_expr: 'tier("a", fixed(0))',
      model_price: 0,
    }
    const token = {
      ...model(4, 'token', 'Alpha', ['all']),
      quota_type: 1,
      billing_mode: 'tiered_expr',
      billing_expr: 'tier("a", p * 2)',
      model_price: 100,
    }
    const input = [fixed, expensive, token, cheap]
    assert.deepEqual(sortModels(input, SORT_OPTIONS.PRICE_LOW), [
      cheap,
      expensive,
      fixed,
      token,
    ])
    assert.deepEqual(sortModels(input, SORT_OPTIONS.PRICE_HIGH), [
      expensive,
      cheap,
      fixed,
      token,
    ])
    assert.deepEqual(input, [fixed, expensive, token, cheap])
  })

  for (const mode of [
    {
      field: 'input_price',
      quota: 0,
      low: SORT_OPTIONS.INPUT_PRICE_LOW,
      high: SORT_OPTIONS.INPUT_PRICE_HIGH,
    },
    {
      field: 'output_price',
      quota: 0,
      low: SORT_OPTIONS.OUTPUT_PRICE_LOW,
      high: SORT_OPTIONS.OUTPUT_PRICE_HIGH,
    },
    {
      field: 'model_price',
      quota: 1,
      low: SORT_OPTIONS.REQUEST_PRICE_LOW,
      high: SORT_OPTIONS.REQUEST_PRICE_HIGH,
    },
  ] as const) {
    test(`${mode.field} sorts both directions with free prices, stable ties, and unknown prices last`, () => {
      const priced = (
        name: string,
        price?: number,
        overrides: Partial<PricingModel> = {}
      ): PricingModel => ({
        ...model(1, name, 'Provider', ['standard']),
        quota_type: mode.quota,
        [mode.field]: price,
        ...overrides,
      })
      const unknown = [
        priced('missing'),
        priced('nan', Number.NaN),
        priced('infinite', Number.POSITIVE_INFINITY),
        priced('negative', -1),
        priced('old schema', 0, { pricing_schema_version: 1 }),
        priced('wrong currency', 0, { pricing_currency: 'CNY' }),
        priced('other unit', 0, { quota_type: mode.quota === 0 ? 1 : 0 }),
        priced('conditional price', 0, {
          billing_mode: 'tiered_expr',
          billing_expr: 'len < 1000 ? tier("a", p * 2) : tier("b", p * 4)',
        }),
        priced('upstream cost', 0, { billing_mode: 'upstream_cost' }),
      ]
      const free = priced('free', 0)
      const firstTie = priced('first tie', 2)
      const secondTie = priced('second tie', 2)
      const expensive = priced('expensive', 10)
      const input = [
        unknown[0],
        expensive,
        firstTie,
        free,
        ...unknown.slice(1),
        secondTie,
      ]
      const snapshot = [...input]
      assert.deepEqual(sortModels(input, mode.low), [
        free,
        firstTie,
        secondTie,
        expensive,
        ...unknown,
      ])
      assert.deepEqual(sortModels(input, mode.high), [
        expensive,
        firstTie,
        secondTie,
        free,
        ...unknown,
      ])
      assert.deepEqual(input, snapshot)
    })

    test(`${mode.field} sorts the selected group's already calculated prices`, () => {
      const prices = (amount: number | null): PricingGroupPrices => ({
        input_price: null,
        output_price: null,
        cache_read_price: null,
        cache_write_price: null,
        image_price: null,
        audio_input_price: null,
        audio_output_price: null,
        model_price: null,
        [mode.field]: amount,
      })
      const first = {
        ...model(1, 'cheap base', 'Provider', ['standard']),
        quota_type: mode.quota,
        [mode.field]: 1,
        group_prices: { standard: prices(10) },
        group_ratio: { standard: 100 },
      }
      const second = {
        ...model(2, 'expensive base', 'Provider', ['standard']),
        quota_type: mode.quota,
        [mode.field]: 20,
        group_prices: { standard: prices(2) },
        group_ratio: { standard: 0.01 },
      }
      const unknown = {
        ...model(3, 'unknown group price', 'Provider', ['standard']),
        quota_type: mode.quota,
        [mode.field]: 0,
        group_prices: { standard: prices(null) },
      }
      for (const [sortBy, expected] of [
        [mode.low, ['expensive base', 'cheap base', 'unknown group price']],
        [mode.high, ['cheap base', 'expensive base', 'unknown group price']],
      ] as const) {
        assert.deepEqual(
          filterAndSortModels([unknown, first, second], {
            ...noOtherFilters,
            group: 'standard',
            vendor: FILTER_ALL,
            sortBy,
          }).map((entry) => entry.model_name),
          expected
        )
      }
    })
  }

  test('input and output sorting use independent USD rates instead of legacy ratios', () => {
    const inputCheap = {
      ...model(1, 'input cheap', 'Provider', ['all']),
      input_price: 1,
      output_price: 10,
      model_ratio: 99,
      completion_ratio: 0,
    }
    const outputCheap = {
      ...model(2, 'output cheap', 'Provider', ['all']),
      input_price: 2,
      output_price: 3,
      model_ratio: 0,
      completion_ratio: 99,
    }
    assert.deepEqual(
      sortModels([inputCheap, outputCheap], SORT_OPTIONS.INPUT_PRICE_LOW),
      [inputCheap, outputCheap]
    )
    assert.deepEqual(
      sortModels([inputCheap, outputCheap], SORT_OPTIONS.OUTPUT_PRICE_LOW),
      [outputCheap, inputCheap]
    )
  })

  test('legacy sort URLs remain accepted and cannot rank unversioned request prices as free', () => {
    const request = {
      ...model(1, 'request', 'Provider', ['all']),
      quota_type: 1,
      model_price: 3,
    }
    const token = { ...model(2, 'token', 'Provider', ['all']), input_price: 2 }
    const unknown = {
      ...request,
      model_name: 'unknown schema',
      pricing_schema_version: undefined,
      model_price: 0,
    }
    assert.deepEqual(sortModels([unknown, request, token], 'price-low'), [
      token,
      request,
      unknown,
    ])
    assert.deepEqual(sortModels([unknown, request, token], 'price-high'), [
      request,
      token,
      unknown,
    ])
  })

  test('combines provider and group filters', () => {
    const models = [
      model(1, 'alpha-standard', 'Alpha', ['standard']),
      model(2, 'alpha-premium', 'Alpha', ['premium']),
      model(3, 'beta-premium', 'Beta', ['premium']),
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      vendor: 'Alpha',
      group: 'premium',
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['alpha-premium']
    )
  })

  test('keeps all-groups models when a specific group is selected', () => {
    const models = [
      model(1, 'alpha-premium', 'Alpha', ['premium']),
      model(2, 'alpha-shared', 'Alpha', ['all']),
      model(3, 'beta-shared', 'Beta', ['all']),
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      vendor: 'Alpha',
      group: 'premium',
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['alpha-premium', 'alpha-shared']
    )
  })

  test('searches supported endpoint types advertised by the search field', () => {
    const models = [
      {
        ...model(1, 'generic-model', 'Provider', ['all']),
        supported_endpoint_types: ['anthropic'],
      },
      {
        ...model(2, 'other-model', 'Provider', ['all']),
        supported_endpoint_types: ['openai'],
      },
    ]

    const result = filterAndSortModels(models, {
      ...noOtherFilters,
      search: 'anthropic',
      vendor: FILTER_ALL,
      group: FILTER_ALL,
    })

    assert.deepEqual(
      result.map((item) => item.model_name),
      ['generic-model']
    )
  })
})
