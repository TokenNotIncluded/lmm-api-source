/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { Settings2, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { DataTableRowActionMenu } from '@/components/data-table/core/row-action-menu'
import { DropdownMenuItem } from '@/components/ui/dropdown-menu'

import type { ApiKey } from '../types'
import { useApiKeys } from './api-keys-provider'

export function DataTableRowActions({ apiKey }: { apiKey: ApiKey }) {
  const { t } = useTranslation()
  const { setCurrentRow, setOpen } = useApiKeys()

  return (
    <DataTableRowActionMenu ariaLabel={t('Open menu')} modal={false}>
      <DropdownMenuItem
        onClick={() => {
          setCurrentRow(apiKey)
          setOpen('update')
        }}
      >
        <Settings2 aria-hidden='true' />
        {t('Configure')}
      </DropdownMenuItem>
      <DropdownMenuItem
        className='text-destructive focus:text-destructive'
        onClick={() => {
          setCurrentRow(apiKey)
          setOpen('delete')
        }}
      >
        <Trash2 aria-hidden='true' />
        {t('Delete')}
      </DropdownMenuItem>
    </DataTableRowActionMenu>
  )
}
