/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { Window } from 'happy-dom'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import { getPricingBasisLabel } from '../lib/pricing-context'
import type { PricingModel } from '../types'
import { ImagePrices } from './image-prices'
import { ModelCard } from './model-card'
import { ModelDetailsContent } from './model-details'
import { PricingContextLabel } from './pricing-context-label'
import { PricingToolbar, type PricingToolbarProps } from './pricing-toolbar'
import { RequestEstimator } from './request-estimator'

test('router cards and details show the exact effective cost factor once and keep budget blocks visible', () => {
  const model: PricingModel = {
    ...groupModel,
    model_name: 'openrouter/fusion',
    billing_mode: 'upstream_cost',
    input_price: null,
    output_price: null,
    group_upstream_cost_prices: {
      standard: {
        currency: 'USD',
        cost_source: 'usage.cost',
        credits_per_usd: 500_000,
        rounding: 'ceil_total_credits',
        rate_basis: 'effective_group_rate',
        multiplier: { numerator: '97', denominator: '80' },
      },
    },
    group_unavailable_reasons: {
      standard: 'upstream_budget_contract_required',
    },
  }
  const card = renderCard(model).textContent
  assert.match(card, /ceil\(USD cost × 500,000 × 97\/80\)/)
  assert.match(card, /Text calls blocked/)
  assert.doesNotMatch(card, /Image prices|Input 0|Free/)
  const details = render(
    <ModelDetailsContent
      model={model}
      groupRatio={{ standard: 10 }}
      usableGroup={{}}
      endpointMap={{}}
      autoGroups={[]}
      tokenUnit='M'
      displayCurrency='USD'
    />
  ).textContent
  assert.doesNotMatch(details, /Account discount|account discount/)
  assert.match(details, /Missing cost stays pending/)
  assert.equal(renderEstimator(model).textContent, '')
})

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
  interpolation: { escapeValue: false },
})
const domWindow = new Window()
const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
})
after(() => {
  queryClient.clear()
  domWindow.close()
})

const groupModel: PricingModel = {
  id: 1,
  model_name: 'group-priced-model',
  quota_type: 0,
  pricing_schema_version: 2,
  pricing_currency: 'USD',
  input_price: 1.94,
  output_price: 3.88,
  cache_read_price: null,
  enable_groups: ['standard'],
  group_ratio: { standard: 10 },
  pricing_context: {
    rate_basis: 'effective_group_rate',
  },
}

const baseImageVariant = {
  endpoint: 'generations',
  size: '1024x1024',
  quality: 'standard',
  max_n: 1,
  count_basis: 'requested',
  usd_micros_per_image: { numerator: '40000', denominator: '1' },
  credits_per_image: { numerator: '20000', denominator: '1' },
  reservation_credits_for_one: 20000,
  rounding: 'ceil_total_credits',
  pricing_version: 1,
  group: 'standard',
  model: 'image-model',
  rate_basis: 'base_group_rate',
} as const

function render(element: React.ReactNode) {
  const container = domWindow.document.createElement('div')
  container.innerHTML = renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <I18nextProvider i18n={i18n}>{element}</I18nextProvider>
    </QueryClientProvider>
  )
  return container
}

function renderCard(model: PricingModel) {
  return render(<ModelCard model={model} onClick={() => {}} />)
}

function renderEstimator(model: PricingModel) {
  return render(
    <RequestEstimator
      model={model}
      groupRatio={{ standard: 10 }}
      usableGroup={{ standard: { desc: 'Standard', ratio: 10 } }}
      endpointMap={{}}
      autoGroups={[]}
      tokenUnit='M'
      displayCurrency='USD'
    />
  )
}

test('public group prices do not invent a personal discount', () => {
  const model: PricingModel = {
    ...groupModel,
    pricing_context: {
      rate_basis: 'base_group_rate',
    },
  }
  const text = renderCard(model).textContent ?? ''
  assert.match(text, /Group base price/)
  assert.doesNotMatch(
    text,
    /Group price|Account discount|100%|not_evaluated|verified/
  )
})

test('the session label describes group pricing without a trust discount', () => {
  const text =
    render(<PricingContextLabel model={groupModel} />).textContent ?? ''
  assert.match(text, /Group price/)
  assert.doesNotMatch(text, /discount|multiplier|trust|verified/i)
})

test('the card uses returned group rates without multiplying either factor again', () => {
  for (const model of [
    groupModel,
    { ...groupModel, group_ratio: { standard: 500 } },
  ]) {
    const text = renderCard(model).textContent ?? ''
    assert.match(text, /Input 1\.94 USD/)
    assert.match(text, /Output 3\.88 USD/)
    assert.doesNotMatch(text, /1\.8818|0\.97 USD|19\.4 USD/)
  }
})

test('a missing group input rate stays unavailable rather than becoming free', () => {
  const model = { ...groupModel, input_price: null }
  const text = renderCard(model).textContent ?? ''
  assert.match(text, /Input -/)
  assert.doesNotMatch(text, /Input 0 USD/)
  assert.match(renderEstimator(model).textContent ?? '', /Estimate unavailable/)
})

test('request estimates use returned group rates once and describe that basis', () => {
  for (const model of [
    groupModel,
    { ...groupModel, group_ratio: { standard: 500 } },
  ]) {
    const estimate = renderEstimator(model)
    assert.equal(
      estimate.querySelector('[aria-live="polite"]')?.textContent,
      '0.02716 USD'
    )
    assert.match(estimate.textContent ?? '', /Group price/)
    assert.match(
      estimate.textContent ?? '',
      /One request using the displayed prices/
    )
    assert.doesNotMatch(estimate.textContent ?? '', /base price \(1×\)/)
  }
})

test('a model without a native pricing context retains its explicit fallback label', () => {
  const model = { ...groupModel, pricing_context: undefined }
  assert.equal(getPricingBasisLabel(model), 'Base price (1×)')
  assert.doesNotMatch(
    renderCard(model).textContent ?? '',
    /Account discount|Group price/
  )
})

test('image-only pricing labels public and session group rates', () => {
  const publicImages: PricingModel = {
    ...groupModel,
    quota_type: 2,
    pricing_context: { rate_basis: 'base_group_rate' },
    image_variants: [baseImageVariant],
  }
  assert.equal(getPricingBasisLabel(publicImages), 'Group base price')
  const sessionImages: PricingModel = {
    ...publicImages,
    pricing_context: { rate_basis: 'effective_group_rate' },
    image_variants: [
      { ...baseImageVariant, rate_basis: 'effective_group_rate' },
    ],
  }
  assert.equal(getPricingBasisLabel(sessionImages), 'Group price')
  assert.doesNotMatch(
    renderCard(sessionImages).textContent ?? '',
    /Account discount/
  )
  assert.equal(
    getPricingBasisLabel({ ...sessionImages, image_variants: [] }),
    'Price'
  )
})

test('image prices preserve returned group rates without a second multiplication', () => {
  const model: PricingModel = {
    ...groupModel,
    image_variants: [
      { ...baseImageVariant, rate_basis: 'effective_group_rate' },
      {
        ...baseImageVariant,
        rate_basis: 'effective_group_rate',
        usd_micros_per_image: { numerator: '38800', denominator: '1' },
        credits_per_image: { numerator: '19400', denominator: '1' },
      },
    ],
  }
  const prices = render(<ImagePrices model={model} />)
  const lists = [...prices.querySelectorAll('dl')]
  assert.equal(lists.length, 2)
  assert.match(lists[0].textContent ?? '', /Group price0\.04 USD/)
  assert.match(lists[0].textContent ?? '', /20000 Credits/)
  assert.match(lists[1].textContent ?? '', /Group price0\.0388 USD/)
  assert.match(lists[1].textContent ?? '', /19400 Credits/)
  assert.doesNotMatch(
    prices.textContent ?? '',
    /Account discount|0\.037636|18818 Credits/
  )
})

test('model details show configured group prices and keep missing rates unavailable', () => {
  const model: PricingModel = {
    ...groupModel,
    performance_available: false,
    group_prices: {
      standard: {
        input_price: 1.94,
        output_price: null,
        cache_read_price: null,
        cache_write_price: null,
        image_price: null,
        audio_input_price: null,
        audio_output_price: null,
        model_price: null,
      },
    },
  }
  const details = render(
    <ModelDetailsContent
      model={model}
      groupRatio={{ standard: 10 }}
      usableGroup={{}}
      endpointMap={{}}
      autoGroups={[]}
      tokenUnit='M'
      displayCurrency='USD'
    />
  )
  const text = details.textContent ?? ''
  assert.match(text, /Group price/)
  assert.doesNotMatch(text, /Account discount|account discount/)
  assert.match(text, /Input1\.94 USD/)
  assert.match(text, /Output-/)
  assert.doesNotMatch(text, /1\.8818 USD|19\.4 USD|base price \(1×\)/)
})

test('toolbar explains a shared group price without claiming an old 1x basis', () => {
  const noChange = () => {}
  const props: PricingToolbarProps = {
    filteredCount: 1,
    sortBy: 'name',
    onSortChange: noChange,
    tokenUnit: 'M',
    onTokenUnitChange: noChange,
    displayCurrency: 'USD',
    onDisplayCurrencyChange: noChange,
    viewMode: 'card',
    onViewModeChange: noChange,
    quotaTypeFilter: 'all',
    endpointTypeFilter: 'all',
    vendorFilter: 'all',
    groupFilter: 'all',
    tagFilter: 'all',
    onQuotaTypeChange: noChange,
    onEndpointTypeChange: noChange,
    onVendorChange: noChange,
    onGroupChange: noChange,
    onTagChange: noChange,
    vendors: [],
    groups: ['standard'],
    tags: [],
    models: [groupModel],
    hasActiveFilters: false,
    activeFilterCount: 0,
    onClearFilters: noChange,
  }
  const toolbar = render(<PricingToolbar {...props} />)
  const note = toolbar.querySelector('[role="note"]')?.textContent ?? ''
  assert.match(note, /Group price/)
  assert.doesNotMatch(note, /Account discount|account discount/)
  assert.doesNotMatch(note, /base price \(1×\)/)
})

test('usage-priced images show an operation reservation without a fictional per-image price', () => {
  const model: PricingModel = {
    ...groupModel,
    image_variants: [
      {
        group: 'standard',
        model: 'image-model',
        endpoint: 'generations',
        size: '1024x1024',
        quality: 'standard',
        max_n: 4,
        count_basis: {
          native_tokens: {
            source_expression: 'tier("standard", p * 5 + img_o * 30)',
            billing_price_key: 'image-model',
            source_sha256: 'a'.repeat(64),
            currency_basis: 'legacy_pricing_unit',
            credits_per_usd: 500000,
            usd_multiplier: { numerator: 1, denominator: 1 },
            max_input_tokens: 4096,
            max_output_tokens: 8192,
          },
        },
        usd_micros_per_image: null,
        credits_per_image: null,
        reservation_credits_for_one: 140000,
        reservation_scope: 'per_operation',
        rounding: 'half_up_total_credits_positive_minimum',
        pricing_version: 4,
      },
    ],
  }
  const page = render(<ImagePrices model={model} />)
  assert.match(page.textContent ?? '', /Usage-based billing/)
  assert.match(page.textContent ?? '', /Reservation per request140000 Credits/)
  assert.match(page.textContent ?? '', /Actual token usage/)
  assert.doesNotMatch(
    page.textContent ?? '',
    /0 USD|140000 Credits \/ Image|Charges round up/
  )
})
