/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import type { StoreProduct, StoreVariant } from './types'

export function enabledStoreVariants(product: StoreProduct): StoreVariant[] {
  return (product.variants || []).filter((variant) => variant.enabled)
}
export function initialStoreVariant(product: StoreProduct): string {
  const variants = enabledStoreVariants(product)
  return variants.length === 1 ? variants[0].id : ''
}
export function selectedStoreVariant(product: StoreProduct, id: string) {
  return enabledStoreVariants(product).find(
    (variant) => variant.id === id && variant.product_id === product.id
  )
}
export function storeVariantPrice(
  product: StoreProduct,
  id: string
): number | undefined {
  const price = selectedStoreVariant(product, id)?.price_quota
  return typeof price === 'number' && Number.isSafeInteger(price) && price > 0
    ? price
    : undefined
}
export function storeVariantCapacity(
  product: StoreProduct,
  id: string
): number {
  const variant = selectedStoreVariant(product, id)
  if (!variant || variant.trading_paused) {
    return 0
  }
  // An unbounded capacity is not a stock count. Checkout applies payment,
  // total-price and purchase limits before passing a finite maximum to controls.
  if (storeVariantUnlimitedSupply(product, id) && product.sale_limit == null) {
    return Infinity
  }
  const capacity = variant.sale_available
  return typeof capacity === 'number' &&
    Number.isSafeInteger(capacity) &&
    capacity >= 0
    ? capacity
    : 0
}

export function storeVariantUnlimitedSupply(product: StoreProduct, id: string) {
  return selectedStoreVariant(product, id)?.unlimited_supply === true
}
