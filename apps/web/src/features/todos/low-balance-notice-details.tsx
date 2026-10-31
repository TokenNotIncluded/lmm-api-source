/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useTranslation } from 'react-i18next'

import type { TodoItem } from './api'
import { isLowBalanceNotice } from './todo-navigation'

function creditUnits(value: unknown, signed = false) {
  const pattern = signed ? /^(0|-?[1-9]\d*)$/ : /^(0|[1-9]\d*)$/
  return typeof value === 'string' && pattern.test(value) ? value : undefined
}

export function LowBalanceNoticeDetails({ item }: { item: TodoItem }) {
  const { t } = useTranslation()
  if (!isLowBalanceNotice(item)) return null

  // These are the notification's captured values, not the current wallet.
  // Keep Core's exact integer strings; display currency settings do not apply.
  const supportedUnit = item.details?.unit === 'credit_500k_usd'
  const balance = supportedUnit
    ? creditUnits(item.details?.balance_units, true)
    : undefined
  const threshold = supportedUnit
    ? creditUnits(item.details?.threshold_units)
    : undefined

  return (
    <span className='text-muted-foreground mt-2 grid gap-1 text-xs leading-5'>
      <span>
        {t('Balance')} ({t('Credits')}):{' '}
        <span className='font-mono [overflow-wrap:anywhere] tabular-nums'>
          {balance ?? t('Unknown')}
        </span>
      </span>
      <span>
        {t('Balance threshold (credit units)')}:{' '}
        <span className='font-mono [overflow-wrap:anywhere] tabular-nums'>
          {threshold ?? t('Unknown')}
        </span>
      </span>
    </span>
  )
}
