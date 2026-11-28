/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { StatusBadge } from '@/components/status-badge'

import type { UsageLog } from '../data/schema'
import { routeProtocolDirection } from '../lib/failure-facts'

import {
  nativeRequestStates,
  requestResultLabels,
  settlementStatusLabels,
} from '../lib/receipt-state'

export function NativeRequestBadges({
  states,
  routeAttempts,
}: {
  routeAttempts?: UsageLog['route_attempts']
  states: NonNullable<ReturnType<typeof nativeRequestStates>>
}) {
  const { t } = useTranslation()
  const { result, settlement } = states
  const directions = new Set(
    routeAttempts?.map(routeProtocolDirection).filter((direction) => direction != null)
  )
  return (
    <div className='flex flex-wrap items-center gap-1 text-xs'>
      <span title={t('Request result')}>
        <StatusBadge
          label={t(result ? requestResultLabels[result] : 'Not recorded')}
          variant={
            result === 'success'
              ? 'success'
              : result === 'error'
                ? 'danger'
                : 'warning'
          }
          size='sm'
          copyable={false}
        />
      </span>
      <span title={t('Settlement status')}>
        <StatusBadge
          label={t(
            settlement ? settlementStatusLabels[settlement] : 'Not recorded'
          )}
          variant={settlement === 'pending' ? 'warning' : 'neutral'}
          size='sm'
          copyable={false}
        />
      </span>
      {[...directions].map((direction) => (
        <span
          key={direction}
          className='text-muted-foreground whitespace-nowrap'
        >
          {direction}
        </span>
      ))}
    </div>
  )
}
