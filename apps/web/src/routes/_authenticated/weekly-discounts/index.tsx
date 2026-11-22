/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute } from '@tanstack/react-router'
import { z } from 'zod'

import { WeeklyDiscounts } from '@/features/weekly-discounts'
import { weeklyDecisionID } from '@/features/weekly-discounts/api'

export const Route = createFileRoute('/_authenticated/weekly-discounts/')({
  validateSearch: z
    .object({ id: z.string().regex(weeklyDecisionID).optional() })
    .strict(),
  component: WeeklyDiscountsRoute,
})
function WeeklyDiscountsRoute() {
  const { id } = Route.useSearch()
  return <WeeklyDiscounts selectedID={id} />
}
