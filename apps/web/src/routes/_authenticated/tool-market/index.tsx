/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { createFileRoute } from '@tanstack/react-router'
import { z } from 'zod'

import { ToolMarket } from '@/features/tool-market'
import { RelayCheckout } from '@/features/tool-market/relay-checkout'
import { parseMarketServiceID } from '@/features/tool-market/service-link'

export const Route = createFileRoute('/_authenticated/tool-market/')({
  validateSearch: z.object({
    relay_order: z
      .string()
      .regex(/^[0-9a-f]{64}$/)
      .catch('')
      .optional(),
    service_id: z.preprocess(parseMarketServiceID, z.string().optional()),
    tab: z.enum(['market', 'connections', 'mine']).catch('market').optional(),
  }),
  component: Page,
})

function Page() {
  const { service_id, tab, relay_order } = Route.useSearch()
  if (relay_order) return <RelayCheckout orderID={relay_order} />
  return <ToolMarket initialServiceID={service_id} initialTab={tab} />
}
