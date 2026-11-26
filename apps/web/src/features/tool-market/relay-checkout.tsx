/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { useMutation, useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import type { z } from 'zod'

import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { formatRelayUSD, readRelayOrder, relayOrderStates } from './relay-order'

export function RelayCheckout({ orderID }: { orderID: string }) {
  const { t, i18n } = useTranslation()
  const userID = useAuthStore((state) => state.auth.user?.id)
  const path = '/api/ssmcpd/orders/' + encodeURIComponent(orderID)
  const order = useQuery({
    queryKey: ['ssmcpd', userID, 'order', orderID],
    enabled: !!userID,
    queryFn: async ({ signal }) =>
      readRelayOrder(
        (await api.get(path, { signal, skipErrorHandler: true })).data,
        orderID,
        userID
      ),
    gcTime: 0,
  })
  const confirm = useMutation({
    mutationFn: async () => {
      const { data } = await api.post(
        path + '/confirm',
        { order_id: orderID },
        { skipErrorHandler: true }
      )
      return readRelayOrder(data, orderID, userID)
    },
    retry: false,
    onSettled: async () => {
      await order.refetch()
    },
  })
  const labels: Record<z.infer<typeof relayOrderStates>, string> = {
    preview: t('Awaiting confirmation'),
    payment_pending: t('Payment pending'),
    grant_pending: t('Relay quota pending'),
    settlement_pending: t('Pending settlement'),
    completed: t('Completed'),
    declined: t('Declined'),
    expired: t('Expired'),
    refund_required: t('Refund required'),
    needs_review: t('Pending review'),
    refunded: t('Refunded'),
  }
  const value = order.data
  const locale = toIntlLocale(i18n.language) ?? 'en-US'
  return (
    <main className='mx-auto max-w-lg space-y-5 p-6'>
      <h1 className='text-2xl font-semibold'>{t('SSMCP relay traffic')}</h1>
      {order.isPending && <p role='status'>{t('Loading...')}</p>}
      {value && (
        <>
          <p>
            {value.gib} GiB ×{' '}
            {formatRelayUSD(value.unit_price_quota_per_gib, locale)}/GiB
          </p>
          <p className='font-medium'>
            {t('Total')} · {formatRelayUSD(value.total_price_quota, locale)} (
            {new Intl.NumberFormat(locale).format(value.total_price_quota)}{' '}
            {t('Credits')})
          </p>
          <p>
            {t('Billing period')} · {value.period}
          </p>
          <p>
            {t('Status')} · {labels[value.state]}
          </p>
          {[
            'preview',
            'payment_pending',
            'grant_pending',
            'settlement_pending',
          ].includes(value.state) && (
            <Button
              disabled={confirm.isPending || order.isFetching || order.isError}
              onClick={() => confirm.mutate()}
            >
              {value.state === 'preview'
                ? t('Pay with account balance')
                : t('Retry')}
            </Button>
          )}
        </>
      )}
      {(order.isError || confirm.isError) && (
        <p role='alert'>
          {t('Could not verify the order. Check its status before retrying.')}
        </p>
      )}
      <Button
        variant='outline'
        disabled={order.isFetching}
        onClick={() => void order.refetch()}
      >
        {t('Refresh')}
      </Button>
    </main>
  )
}
