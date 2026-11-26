/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import type { NativeChannel } from '@/features/wallet/lib/native-checkout'
import { getNativePaymentChannels } from '@/features/wallet/native-checkout-api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { NativeEPaySettings } from './native-epay-settings'
import { NativePaymentSettings } from './native-payment-settings'
import { NativeProviderGatewaySettings } from './native-provider-gateway-settings'

function methodKey(channel: NativeChannel): string {
  if (channel.provider !== 'epay') return channel.provider
  if (channel.payment_type === 'epay' && channel.currency !== 'LDC') {
    return 'epay/custom/epay'
  }
  return `epay/${channel.payment_type ?? ''}`
}

type Scope = { userId: number; sessionId: string }

function WalletMethods({ userId, sessionId }: Scope) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['payment-settings-wallet-methods', userId, sessionId],
    queryFn: ({ signal }) => getNativePaymentChannels(signal),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  const labels = new Map([
    ['epay/alipay', t('Epay Alipay')],
    ['epay/wxpay', t('Epay WeChat Pay')],
    ['epay/epay', t('LINUX DO Credit')],
    ['stripe', t('Stripe')],
    ['creem', t('Creem')],
    ['waffo', t('Waffo Global')],
    ['waffo_pancake', t('Waffo Pancake')],
    ['coinbase', t('Coinbase Business Checkout')],
  ])
  const byMethod = new Map<string, NativeChannel[]>()
  for (const channel of query.isSuccess && query.data.available
    ? query.data.channels
    : []) {
    const key = methodKey(channel)
    if (!labels.has(key)) {
      labels.set(key, channel.payment_type || t('Epay Gateway'))
    }
    const group = byMethod.get(key)
    if (group) group.push(channel)
    else byMethod.set(key, [channel])
  }

  const noChannel = query.isPending
    ? t('Loading payment methods…')
    : query.isError
      ? t('Availability unknown')
      : t('Not offered to this wallet')

  return (
    <div className='space-y-6'>
      <div className='flex flex-wrap items-center justify-between gap-2'>
        <h2 className='text-base font-medium'>{t('Wallet payment methods')}</h2>
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
      <p className='text-muted-foreground text-sm'>
        {t(
          'Wallet availability is shown for the current account. Manage Pancake and EPay connections in their tabs.'
        )}
      </p>
      {query.isError && (
        <p role='status' className='text-sm'>
          {t('Unable to load payment methods')}
        </p>
      )}
      <dl className='divide-y'>
        {Array.from(labels, ([key, name]) => (
          <div key={key} className='grid gap-1 py-3 sm:grid-cols-2 sm:gap-4'>
            <dt className='font-medium'>{name}</dt>
            <dd className='text-muted-foreground text-sm'>
              {byMethod.get(key)?.map((channel) => (
                <div key={channel.id}>
                  #{channel.id} ·{' '}
                  {channel.provider === 'coinbase' ? 'USDC' : channel.currency}{' '}
                  ·{' '}
                  {channel.environment === 'test'
                    ? t('Test payment')
                    : t('Production')}
                </div>
              )) ?? noChannel}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  )
}

export function NativePaymentGateways() {
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const { t } = useTranslation()
  if (!scope || scope.role !== ROLE.SUPER_ADMIN) {
    return (
      <ErrorState
        title={t('Owner access required')}
        description={t(
          'Only the platform owner can manage payment credentials and gateway settings.'
        )}
      />
    )
  }
  return (
    <GatewayPanels key={`${scope.userId}:${scope.sessionId}`} scope={scope} />
  )
}

function GatewayPanels({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const providers = [
    { id: 'stripe', label: t('Stripe') },
    { id: 'creem', label: t('Creem') },
    { id: 'waffo', label: t('Waffo Global') },
    { id: 'coinbase', label: t('Coinbase Business Checkout') },
  ] as const
  const [opened, setOpened] = useState({ pancake: false, epay: false })
  return (
    <Tabs
      defaultValue='wallet'
      className='gap-8'
      onValueChange={(tab: unknown) => {
        if (tab === 'epay' || tab === 'pancake') {
          setOpened((old) => (old[tab] ? old : { ...old, [tab]: true }))
        }
      }}
    >
      <TabsList variant='navigation' aria-label={t('Payment Gateway')}>
        <TabsTrigger value='wallet'>{t('Wallet payment methods')}</TabsTrigger>
        <TabsTrigger value='pancake'>{t('Waffo Pancake')}</TabsTrigger>
        <TabsTrigger value='epay'>{t('Epay Gateway')}</TabsTrigger>
        {providers.map(({ id, label }) => (
          <TabsTrigger key={id} value={id}>
            {label}
          </TabsTrigger>
        ))}
      </TabsList>
      <TabsContent value='wallet'>
        <WalletMethods userId={scope.userId} sessionId={scope.sessionId} />
      </TabsContent>
      <TabsContent value='pancake' keepMounted={opened.pancake}>
        <NativePaymentSettings />
      </TabsContent>
      <TabsContent value='epay' keepMounted={opened.epay}>
        <NativeEPaySettings />
      </TabsContent>
      {providers.map(({ id }) => (
        <TabsContent key={id} value={id}>
          <NativeProviderGatewaySettings provider={id} scope={scope} />
        </TabsContent>
      ))}
    </Tabs>
  )
}
