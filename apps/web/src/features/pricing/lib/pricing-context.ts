/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { PricingModel } from '../types'

function getPricingBasis(model: PricingModel) {
  if (model.quota_type !== 2) return model.pricing_context?.rate_basis
  const firstVariant = model.image_variants?.[0]
  return firstVariant?.rate_basis &&
    model.image_variants?.every(
      (variant) => variant.rate_basis === firstVariant.rate_basis
    )
    ? firstVariant.rate_basis
    : undefined
}

export function hasCurrentPricingContext(model: PricingModel): boolean {
  return (
    getPricingBasis(model) === 'base_group_rate' ||
    getPricingBasis(model) === 'effective_group_rate' ||
    (model.quota_type === 2 &&
      (model.pricing_context?.rate_basis === 'base_group_rate' ||
        model.pricing_context?.rate_basis === 'effective_group_rate' ||
        Boolean(model.image_variants?.some((variant) => variant.rate_basis))))
  )
}

export function getPricingBasisLabel(
  model: PricingModel,
  legacyLabel = 'Base price (1×)'
): string {
  if (!hasCurrentPricingContext(model)) return legacyLabel
  if (model.quota_type === 2 && getPricingBasis(model) === undefined) {
    return model.image_variants?.length ? 'Image prices' : 'Price'
  }
  return getPricingBasis(model) === 'effective_group_rate'
    ? 'Group price'
    : 'Group base price'
}
