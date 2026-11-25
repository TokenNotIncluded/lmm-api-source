/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { readNativeUsersScope } from '@/features/native-users/access'
import { paymentProviderNames } from '@/features/wallet/lib/payment-providers'
import { getNativePaymentChannels } from '@/features/wallet/native-checkout-api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

const docs = 'https://github.com/TokenNotIncluded/lmm-api-source/blob/main'
const files = ['core-payments.example.toml', 'extensions-payments.example.toml']

export function PaymentProviderStatus() {
  const { t } = useTranslation()
  const scope = readNativeUsersScope(useAuthStore((state) => state.auth))
  const owner = scope?.role === ROLE.SUPER_ADMIN
  const query = useQuery({
    queryKey: [
      'payment-provider-wallet-status',
      scope?.userId,
      scope?.sessionId,
    ],
    enabled: owner,
    queryFn: async ({ signal }) => {
      const data = await getNativePaymentChannels(signal)
      const hasChannels = data.channels.length > 0
      if (data.available !== hasChannels) {
        throw new Error('Inconsistent wallet channel catalogue')
      }
      return data.channels
    },
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  if (!owner) return null
  const channels = query.isSuccess ? query.data : undefined
  return (
    <section
      aria-labelledby='wallet-payment-providers'
      className='max-w-3xl space-y-4'
    >
      <div className='flex items-baseline justify-between gap-4'>
        <h3 id='wallet-payment-providers' className='font-medium'>
          {t('Wallet')} · {t('Payment channels')}
        </h3>
        <Button
          size='sm'
          variant='ghost'
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
        >
          {query.isError ? t('Retry') : t('Refresh')}
        </Button>
      </div>
      {query.isError ? (
        <p role='alert'>{t('Unable to load payment methods')}</p>
      ) : query.isPending ? (
        <p role='status'>{t('Loading payment methods…')}</p>
      ) : null}
      <dl className='divide-border/50 divide-y text-sm'>
        {Object.entries(paymentProviderNames).map(([provider, label]) => {
          const matches = channels?.filter(
            (channel) => channel.provider === provider
          )
          return (
            <div
              key={provider}
              className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-3 py-2'
            >
              <dt>{label}</dt>
              <dd className='text-muted-foreground'>
                {matches === undefined ? (
                  t('Unknown')
                ) : matches.length === 0 ? (
                  t('Not available')
                ) : (
                  <ul>
                    {matches.map((channel) => (
                      <li key={channel.id}>
                        #{channel.id} · {channel.currency} ·{' '}
                        {channel.environment === 'test' ? t('Test') : t('Live')}
                        {channel.payment_type && ` · ${channel.payment_type}`}
                      </li>
                    ))}
                  </ul>
                )}
              </dd>
            </div>
          )
        })}
      </dl>
      <details className='text-muted-foreground text-sm'>
        <summary className='cursor-pointer'>{t('Configuration File')}</summary>
        <ul className='mt-2 space-y-1'>
          {files.map((file) => (
            <li key={file}>
              <a
                href={`${docs}/config/${file}`}
                target='_blank'
                rel='noreferrer'
                className='break-all underline'
              >
                {file}
              </a>
            </li>
          ))}
          <li>
            <a
              href={`${docs}/docs/payment-gateways.md`}
              target='_blank'
              rel='noreferrer'
              className='underline'
            >
              {t('Documentation')}
            </a>
          </li>
        </ul>
      </details>
    </section>
  )
}
