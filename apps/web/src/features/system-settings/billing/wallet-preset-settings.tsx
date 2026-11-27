/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import {
  nativeAmountDisplay,
  nativeAmountMinor,
} from '@/features/wallet/lib/native-checkout'
import { api } from '@/lib/api'

const endpoint = '/api/user/payments/presets'
const currencies = ['USD', 'CNY', 'EUR', 'LDC', 'USDC'] as const
type Currency = (typeof currencies)[number]
const amounts = z
  .array(z.string().regex(/^[1-9]\d{0,12}$/))
  .max(32)
  .refine((values) =>
    values.every((v) => v.length < 13 || v <= '9000000000000')
  )
  .refine((values) => new Set(values).size === values.length)
const presets = z.partialRecord(z.enum(currencies), amounts)
const readResult = z.object({ success: z.literal(true), data: presets })
const savedResult = z.object({ success: z.literal(true), data: amounts })

export function WalletPresetSettings({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const [expanded, setExpanded] = useState(false)
  const queryKey = ['wallet-preset-settings', scope.userId, scope.sessionId]
  const query = useQuery({
    queryKey,
    enabled: expanded,
    queryFn: async ({ signal }) => {
      const response = await api.get<unknown>(
        endpoint,
        nativeUsersOptions(scope, signal)
      )
      return readResult.parse(response.data).data
    },
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  const [currency, setCurrency] = useState<Currency>('USD')
  const [drafts, setDrafts] = useState<Partial<Record<Currency, string>>>({})
  const [state, setState] = useState<'idle' | 'saving' | 'saved' | 'error'>(
    'idle'
  )
  const pending = useRef<AbortController | null>(null)
  useEffect(() => () => pending.current?.abort(), [])

  const saved = query.data?.[currency]
  const value =
    drafts[currency] ?? saved?.map(nativeAmountDisplay).join(', ') ?? ''
  const parsed = amounts.safeParse(
    value.trim() ? value.split(/[,，]/u).map(nativeAmountMinor) : []
  )
  const changed =
    parsed.success && JSON.stringify(parsed.data) !== JSON.stringify(saved)

  async function save() {
    if (
      !query.isSuccess ||
      !isCurrentNativeUsersScope(scope) ||
      !parsed.success ||
      !changed ||
      pending.current
    ) {
      return
    }
    const controller = new AbortController()
    pending.current = controller
    setState('saving')
    try {
      const response = await api.put<unknown>(
        `${endpoint}/${currency}`,
        { amounts_minor: parsed.data },
        nativeUsersOptions(scope, controller.signal)
      )
      const result = savedResult.parse(response.data).data
      if (!controller.signal.aborted && isCurrentNativeUsersScope(scope)) {
        client.setQueryData<z.infer<typeof presets>>(queryKey, (prior) => ({
          ...prior,
          [currency]: result,
        }))
        setDrafts((prior) => ({ ...prior, [currency]: undefined }))
        setState('saved')
      }
    } catch {
      if (!controller.signal.aborted && isCurrentNativeUsersScope(scope)) {
        setState('error')
        await query.refetch()
      }
    } finally {
      pending.current = null
    }
  }

  return (
    <section className='max-w-2xl space-y-3'>
      <Button
        type='button'
        variant='outline'
        aria-expanded={expanded}
        aria-controls='wallet-presets-editor'
        disabled={state === 'saving'}
        onClick={() => setExpanded((open) => !open)}
      >
        {t('Wallet top-up presets')}
      </Button>
      <div id='wallet-presets-editor' hidden={!expanded}>
        {!expanded ? null : query.isPending ? (
          <p role='status'>{t('Loading payment configuration...')}</p>
        ) : query.isError ? (
          <div className='space-y-2'>
            <p role='alert'>{t('Unable to load wallet presets.')}</p>
            <Button variant='outline' onClick={() => void query.refetch()}>
              {t('Retry')}
            </Button>
          </div>
        ) : (
          <form
            className='space-y-3'
            onSubmit={(event) => {
              event.preventDefault()
              void save()
            }}
          >
            <p
              className='text-muted-foreground text-sm'
              id='wallet-presets-hint'
            >
              {t(
                'Separate amounts with commas and use a dot for decimals. An empty list hides the preset buttons. Customers can still enter an amount.'
              )}
            </p>
            <div className='flex flex-wrap items-end gap-3'>
              <div className='space-y-2'>
                <Label htmlFor='wallet-presets-currency'>{t('Currency')}</Label>
                <NativeSelect
                  id='wallet-presets-currency'
                  value={currency}
                  disabled={state === 'saving'}
                  onChange={(event) => {
                    setCurrency(event.target.value as Currency)
                    setState('idle')
                  }}
                >
                  {currencies.map((code) => (
                    <NativeSelectOption key={code} value={code}>
                      {code}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </div>
              <div className='min-w-48 flex-1 space-y-2'>
                <Label htmlFor='wallet-presets-amounts'>
                  {t('Amount')} · {currency}
                </Label>
                <Input
                  id='wallet-presets-amounts'
                  value={value}
                  maxLength={640}
                  placeholder={
                    saved === undefined ? t('Channel defaults') : undefined
                  }
                  aria-invalid={!parsed.success}
                  aria-describedby='wallet-presets-hint'
                  disabled={state === 'saving'}
                  onChange={(event) => {
                    const text = event.target.value
                    setDrafts((prior) => ({ ...prior, [currency]: text }))
                    setState('idle')
                  }}
                />
              </div>
              <Button type='submit' disabled={state === 'saving' || !changed}>
                {t('Save')}
              </Button>
            </div>
            {!parsed.success && (
              <p role='alert' className='text-destructive text-sm'>
                {t(
                  'Enter up to 32 distinct amounts from 0.01 to 90,000,000,000 with at most two decimal places.'
                )}
              </p>
            )}
            {state === 'saved' && <p role='status'>{t('Settings saved')}</p>}
            {state === 'error' && (
              <p role='alert'>
                {t(
                  'The change could not be confirmed. Review the latest status before trying again.'
                )}
              </p>
            )}
          </form>
        )}
      </div>
    </section>
  )
}
