/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import { getPricingBasisLabel } from '../lib/pricing-context'
import type { PricingModel } from '../types'

export function PricingContextLabel(props: {
  model: PricingModel
  legacyLabel?: string
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <span className={cn('block', props.className)}>
      <span>{t(getPricingBasisLabel(props.model, props.legacyLabel))}</span>
    </span>
  )
}
