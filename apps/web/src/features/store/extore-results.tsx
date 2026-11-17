/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import type { ExtoreSyncResult } from './extore-types'

export function StoreExtoreResults({
  results,
}: {
  results: ExtoreSyncResult[]
}) {
  const { t } = useTranslation()
  const succeeded = results.filter((item) => item.success).length
  return (
    <div className='space-y-3' role='status' aria-live='polite'>
      <p className='text-sm font-medium'>
        {t('{{succeeded}} synced · {{failed}} need attention', {
          succeeded,
          failed: results.length - succeeded,
        })}
      </p>
      {!results.length && (
        <p className='text-muted-foreground text-sm'>
          {t(
            'No products were authorized. Connect again and select products in Extore.'
          )}
        </p>
      )}
      <ul className='divide-border/50 divide-y'>
        {results.map((item) => (
          <li
            key={item.source_id}
            className='flex flex-wrap items-center justify-between gap-3 py-3'
          >
            <div className='min-w-0 space-y-1'>
              <p className='text-sm font-medium break-words'>
                {item.source_id}
              </p>
              <p
                className={
                  item.success
                    ? 'text-muted-foreground text-xs'
                    : 'text-destructive text-xs'
                }
              >
                {t(item.message)}
              </p>
            </div>
            {item.success && item.product_id && (
              <Button
                size='sm'
                variant='ghost'
                render={
                  <a
                    href={`/store/preview/${encodeURIComponent(item.product_id)}`}
                  />
                }
              >
                {t('View in store')}
              </Button>
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}
