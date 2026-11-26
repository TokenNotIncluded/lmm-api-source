/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

const providerSchema = z.enum(['stripe', 'creem', 'waffo', 'coinbase'])
export type GatewayProvider = z.infer<typeof providerSchema>

const bindingSchema = z.object({
  channel_id: z.string().regex(/^[1-9][0-9]*$/),
  provider: providerSchema,
  merchant_id: z.string(),
  environment: z.enum(['test', 'live']),
  currency: z.string(),
  wallet_enabled: z.boolean(),
  subscription_enabled: z.boolean().optional(),
  api_version: z.string().optional(),
  api_key_configured: z.boolean(),
  private_key_configured: z.boolean().optional(),
  product_id: z.string().optional(),
  product_amount_minor: z
    .string()
    .regex(/^[0-9]+$/)
    .optional(),
  pay_method_type: z.string().optional(),
  pay_method_name: z.string().optional(),
  success_url: z.string().optional(),
  cancel_url: z.string().optional(),
  notify_url: z.string().optional(),
  return_url: z.string().optional(),
})

const responseSchema = z.object({
  success: z.literal(true),
  data: z.object({
    contract: z.literal('extensions.provider-gateways.v1'),
    callbacks_enabled: z.boolean(),
    bindings: z.array(bindingSchema),
  }),
})

const configExamples =
  'https://github.com/TokenNotIncluded/lmm-api-source/blob/main/config/'

async function readConfiguration(scope: NativeUsersScope, signal: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope) || signal.aborted) {
    throw new Error('Expired payment configuration scope')
  }
  const response = await api.get<unknown>(
    '/api/option/payment-gateways/config',
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope) || signal.aborted) {
    throw new Error('Expired payment configuration scope')
  }
  return responseSchema.parse(response.data).data
}

export function NativeProviderGatewaySettings({
  provider,
  scope,
}: {
  provider: GatewayProvider
  scope: NativeUsersScope
}) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['payment-provider-configuration', scope.userId, scope.sessionId],
    queryFn: ({ signal }) => readConfiguration(scope, signal),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })

  if (query.isPending) {
    return <p role='status'>{t('Loading payment configuration...')}</p>
  }
  if (query.isError) {
    return (
      <ErrorState
        title={t('Payment configuration unavailable')}
        description={t(
          'The payment service could not provide its configuration. Check the service and host setup, then retry.'
        )}
        onRetry={() => void query.refetch()}
      />
    )
  }

  const bindings = query.data.bindings.filter(
    (binding) => binding.provider === provider
  )
  const enabled = (value: boolean) => (value ? t('Enabled') : t('Disabled'))
  const configured = (value: boolean) =>
    value ? t('Configured') : t('Not configured')

  return (
    <div className='space-y-6'>
      <div className='flex flex-wrap items-center justify-between gap-2'>
        <h2 className='text-base font-medium'>{t('Host setup')}</h2>
        <Button
          type='button'
          variant='outline'
          size='sm'
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
        >
          {t('Refresh')}
        </Button>
      </div>
      <div className='space-y-2 text-sm'>
        <p className='text-muted-foreground'>
          {t(
            'For Stripe, Creem, Waffo Global and Coinbase, configure matching channels in Core and Extensions.'
          )}
        </p>
        <div className='flex flex-wrap gap-x-5 gap-y-2'>
          {[
            'core-payments.example.toml',
            'extensions-payments.example.toml',
          ].map((filename) => (
            <a
              key={filename}
              className='underline underline-offset-4'
              href={configExamples + filename}
              target='_blank'
              rel='noreferrer'
            >
              {filename}
            </a>
          ))}
        </div>
      </div>
      {bindings.length === 0 && <p>{t('No payment channel is configured.')}</p>}
      {bindings.length > 0 && (
        <p className='text-muted-foreground text-sm'>
          {t('Payment notifications')} · {enabled(query.data.callbacks_enabled)}
        </p>
      )}
      {bindings.map((binding) => {
        const rows: [string, string | undefined][] = [
          [t('Merchant ID'), binding.merchant_id],
          [
            t('Environment'),
            binding.environment === 'test'
              ? t('Test payment')
              : t('Production'),
          ],
          [t('Currency'), binding.currency],
          [t('Wallet checkout'), enabled(binding.wallet_enabled)],
          [
            t('Subscription checkout'),
            binding.subscription_enabled === undefined
              ? undefined
              : enabled(binding.subscription_enabled),
          ],
          [t('API version'), binding.api_version],
          [t('API key'), configured(binding.api_key_configured)],
          [
            t('Private key'),
            binding.private_key_configured === undefined
              ? undefined
              : configured(binding.private_key_configured),
          ],
          [t('Product ID'), binding.product_id],
          [
            t('Product price'),
            binding.product_amount_minor &&
              formatAmount(binding.product_amount_minor, binding.currency),
          ],
          [t('Payment method type'), binding.pay_method_type],
          [t('Payment method name'), binding.pay_method_name],
          [t('Checkout success URL'), binding.success_url],
          [t('Checkout cancellation URL'), binding.cancel_url],
          [t('Webhook URL'), binding.notify_url],
          [t('Payment return URL'), binding.return_url],
        ]
        return (
          <section className='space-y-2' key={binding.channel_id}>
            <h3 className='font-medium'>#{binding.channel_id}</h3>
            <dl className='divide-y text-sm'>
              {rows
                .filter(([, value]) => value !== undefined)
                .map(([label, value]) => (
                  <div
                    className='grid gap-1 py-2 sm:grid-cols-2 sm:gap-4'
                    key={label}
                  >
                    <dt>{label}</dt>
                    <dd className='text-muted-foreground break-all'>{value}</dd>
                  </div>
                ))}
            </dl>
          </section>
        )
      })}
    </div>
  )
}

function formatAmount(value: string, currency: string) {
  const minor = BigInt(value)
  return `${minor / 100n}.${(minor % 100n).toString().padStart(2, '0')} ${currency}`
}
