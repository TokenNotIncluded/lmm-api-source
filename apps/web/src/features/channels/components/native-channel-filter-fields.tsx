/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import type { NativeChannelFilters } from '../native-channel-statistics'

export function NativeChannelFilterFields({
  value,
  groups,
  disabled,
  onChange,
}: {
  value: NativeChannelFilters
  groups: string[]
  disabled: boolean
  onChange: (value: NativeChannelFilters) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const selectClass =
    'border-input bg-background block min-h-11 w-full min-w-0 rounded-md border px-3 text-sm'
  return (
    <div className='grid w-full min-w-0 grid-cols-2 gap-3'>
      <label htmlFor={`${id}-status`} className='min-w-0 space-y-2 text-sm'>
        <span>{t('Status')}</span>
        <select
          id={`${id}-status`}
          className={selectClass}
          value={value.status}
          disabled={disabled}
          onChange={(event) =>
            onChange({
              ...value,
              status: event.target.value as NativeChannelFilters['status'],
            })
          }
        >
          <option value='all'>{t('All statuses')}</option>
          <option value='enabled'>{t('Enabled')}</option>
          <option value='disabled'>{t('Disabled')}</option>
        </select>
      </label>
      <label htmlFor={`${id}-group`} className='min-w-0 space-y-2 text-sm'>
        <span>{t('Group')}</span>
        <select
          id={`${id}-group`}
          className={selectClass}
          value={value.group}
          disabled={disabled}
          onChange={(event) =>
            onChange({ ...value, group: event.target.value })
          }
        >
          <option value=''>{t('All groups')}</option>
          {[...new Set([...groups, ...(value.group ? [value.group] : [])])].map(
            (group) => (
              <option key={group} value={group}>
                {group}
              </option>
            )
          )}
        </select>
      </label>
    </div>
  )
}
