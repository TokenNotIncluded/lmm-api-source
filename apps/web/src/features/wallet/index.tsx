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
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useAuthUserRefresh } from '@/features/onboarding'
import { NativeWalletPromotions } from '@/features/promotions/native-wallet'
import { NativeWalletSubscriptions } from '@/features/subscriptions/native-wallet'
import { useAuthStore } from '@/stores/auth-store'

import { AffiliateRewardsCard } from './components/affiliate-rewards-card'
import { BillingHistoryDialog } from './components/dialogs/billing-history-dialog'
import { NativeWalletCheckout } from './components/native-wallet-checkout'
import { WalletStatsCard } from './components/wallet-stats-card'
import { useCheckoutOwnerKey } from './hooks/use-checkout-scope'
import { walletProjection } from './lib/wallet-projection'
import type { UserWalletData } from './types'

interface WalletProps {
  initialShowHistory?: boolean
  initialTopupAmount?: number
  paymentReturn?: 'return' | 'cancel'
}

export function Wallet(props: WalletProps) {
  const ownerKey = useCheckoutOwnerKey()
  return <NativeWallet key={ownerKey} {...props} />
}

function NativeWallet({ initialShowHistory = false }: WalletProps) {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user) as UserWalletData | null
  const projection = walletProjection(user)
  const nativeCreditsAvailable =
    user?.financial_projection_available === true &&
    projection.available !== null
  // Credit-denominated subscriptions and rewards do not require a fiat basis.
  // These facts permit loading the native workspaces, not authorizing a write:
  // their APIs still enforce the current session, plan, campaign and balance.
  // Recharge separately requires its provider catalog and explicit quote.
  const { refreshUser } = useAuthUserRefresh()
  const [historyOpen, setHistoryOpen] = useState(initialShowHistory)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshFailed, setRefreshFailed] = useState(false)
  const refresh = async () => {
    setRefreshing(true)
    setRefreshFailed(!(await refreshUser()))
    setRefreshing(false)
  }

  return (
    <>
      <SectionPageLayout>
        <SectionPageLayout.Title>
          <span className='whitespace-nowrap'>{t('Wallet')}</span>
        </SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <div className='flex max-w-[70vw] flex-wrap justify-end gap-2'>
            <Button variant='outline' onClick={() => setHistoryOpen(true)}>
              {t('Order History')}
            </Button>
            <Button
              variant='outline'
              disabled={refreshing}
              onClick={() => void refresh()}
            >
              {t('Refresh account status')}
            </Button>
          </div>
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='wallet-editorial mx-auto flex w-full max-w-5xl flex-col gap-7 sm:gap-9'>
            {!nativeCreditsAvailable ? (
              <Alert>
                <AlertDescription>
                  {t(
                    'Wallet data is unavailable. Payments and transfers are disabled until it is restored.'
                  )}
                </AlertDescription>
              </Alert>
            ) : null}
            {refreshFailed ? <p role='alert'>{t('Refresh failed')}</p> : null}
            <WalletStatsCard user={user} />
            {nativeCreditsAvailable ? <NativeWalletCheckout /> : null}
            {nativeCreditsAvailable ? (
              <NativeWalletSubscriptions onUpdated={() => void refresh()} />
            ) : null}
            {nativeCreditsAvailable ? (
              <NativeWalletPromotions onUpdated={() => void refresh()} />
            ) : null}
            <div className='flex flex-wrap gap-2'>
              {!nativeCreditsAvailable ? (
                <Button disabled>{t('Add funds')}</Button>
              ) : null}
              {!nativeCreditsAvailable ? (
                <Button variant='outline' disabled>
                  {t('Subscriptions')}
                </Button>
              ) : null}
              <Button variant='outline' disabled>
                {t('Transfer')}
              </Button>
            </div>
            <AffiliateRewardsCard user={user} />
          </div>
        </SectionPageLayout.Content>
      </SectionPageLayout>
      <BillingHistoryDialog open={historyOpen} onOpenChange={setHistoryOpen} />
    </>
  )
}
