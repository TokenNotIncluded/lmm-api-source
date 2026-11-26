/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { microsToUsd } from '@/features/channels/components/native-model-draft'

import {
  fetchModelsDevPrices,
  type CatalogRates,
} from './native-pricing-sources'

export function NativeUpstreamPriceSync({
  model,
  defaultOpen = false,
  onApply,
}: {
  model: string
  defaultOpen?: boolean
  onApply: (rates: CatalogRates) => void
}) {
  const { t } = useTranslation()
  const [modelID, setModelID] = useState(model)
  const [requestedModel, setRequestedModel] = useState('')
  const [provider, setProvider] = useState('')
  const query = useQuery({
    queryKey: ['native-models-dev-prices', requestedModel],
    queryFn: ({ signal }) => fetchModelsDevPrices(requestedModel, signal),
    enabled: Boolean(requestedModel),
    retry: false,
    refetchOnWindowFocus: false,
    gcTime: 0,
  })
  const entry = query.data?.find((item) => item.provider === provider)
  const exactRequest = modelID.trim() === requestedModel
  return (
    <details
      open={defaultOpen || undefined}
      className='space-y-3 rounded-lg border p-3'
    >
      <summary className='cursor-pointer text-sm font-medium'>
        {t('Upstream price sync')}
      </summary>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Load public USD prices from models.dev and choose a provider. Review the values before applying the draft.'
        )}
      </p>
      <div className='flex flex-wrap items-end gap-2'>
        <label className='grow space-y-1 text-sm'>
          <span>{t('Model ID')}</span>
          <Input
            value={modelID}
            onChange={(event) => {
              setModelID(event.target.value)
              setProvider('')
            }}
            maxLength={200}
          />
        </label>
        <Button
          type='button'
          variant='outline'
          disabled={!modelID.trim() || query.isFetching}
          onClick={() => {
            setProvider('')
            if (exactRequest) void query.refetch()
            else setRequestedModel(modelID.trim())
          }}
        >
          {query.isFetching ? t('Loading...') : t('Fetch upstream prices')}
        </Button>
      </div>
      {exactRequest && query.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Failed to fetch upstream prices')}
        </p>
      )}
      {exactRequest &&
        query.data &&
        (query.data.length ? (
          <label className='block space-y-1 text-sm'>
            <span>{t('Provider')}</span>
            <NativeSelect
              value={provider}
              onChange={(event) => setProvider(event.target.value)}
            >
              <NativeSelectOption value=''>
                {t('Select provider')}
              </NativeSelectOption>
              {query.data.map((item) => (
                <NativeSelectOption key={item.provider} value={item.provider}>
                  {item.name} ({item.provider})
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </label>
        ) : (
          <p role='status' className='text-sm'>
            {t('No matching upstream prices')}
          </p>
        ))}
      {exactRequest && entry && (
        <div className='space-y-2'>
          <p className='text-sm'>
            {t('Input price (USD / 1M tokens)')}:{' '}
            {microsToUsd(entry.rates.input ?? 0)} ·{' '}
            {t('Output price (USD / 1M tokens)')}:{' '}
            {microsToUsd(entry.rates.output ?? 0)}
          </p>
          <Button
            type='button'
            variant='outline'
            disabled={query.isFetching || query.isError}
            onClick={() => onApply(entry.rates)}
          >
            {t('Apply upstream prices')}
          </Button>
        </div>
      )}
    </details>
  )
}
