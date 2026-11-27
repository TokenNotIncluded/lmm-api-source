/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { StatusBadge } from '@/components/status-badge'

import {
  nativeRequestStates,
  requestResultLabels,
  settlementStatusLabels,
} from '../lib/receipt-state'

export function NativeRequestBadges({
  states,
  conversion,
}: {
  conversion?: boolean
  states: NonNullable<ReturnType<typeof nativeRequestStates>>
}) {
  const { t } = useTranslation()
  const { result, settlement } = states
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
      {conversion && (
        <span className='text-muted-foreground'>
          {t('Cross-protocol conversion')}
        </span>
      )}
    </div>
  )
}
