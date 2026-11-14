/*
Copyright (C) 2026 LIghtJUNction
*/
import type {
  WaffoPancakeProductType,
  WaffoPancakePurchaseOption,
} from '../types'

// An empty options list is authoritative. Never fall back to a stale quote.
// With two options, even if only one currency is usable, no automatic switch
// from one-time purchase to auto-renewal is permitted.
export function selectPancakePurchaseOption(
  options: WaffoPancakePurchaseOption[] | undefined,
  selectedType: WaffoPancakeProductType | ''
): WaffoPancakePurchaseOption | undefined {
  if (!options) return undefined
  if (selectedType) {
    return options.find((option) => option.product_type === selectedType)
  }
  return options.length === 1 ? options[0] : undefined
}
