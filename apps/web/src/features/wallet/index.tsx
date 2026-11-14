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
import { History, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { useAuthUserRefresh } from '@/features/onboarding'
import { NativeWalletPromotions } from '@/features/promotions/native-wallet'
import { NativeWalletSubscriptions } from '@/features/subscriptions/native-wallet'
import { isConsoleActivated } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

import { AffiliateRewardsCard } from './components/affiliate-rewards-card'
import { BillingHistoryDialog } from './components/dialogs/billing-history-dialog'
import { L0UpgradePanel } from './components/l0-upgrade-panel'
import { NativeWalletCheckout } from './components/native-wallet-checkout'
import { WalletStatsCard } from './components/wallet-stats-card'
import { useCheckoutOwnerKey } from './hooks/use-checkout-scope'
import { walletProjection } from './lib/wallet-projection'
import type { UserWalletData } from './types'

import './wallet-layout.css'

interface WalletProps {
  initialShowHistory?: boolean
  initialTopupAmount?: number
  paymentReturn?: 'return' | 'cancel'
  preview?: boolean
}

export function Wallet(props: WalletProps) {
  const ownerKey = useCheckoutOwnerKey()
  return <NativeWallet key={ownerKey} {...props} />
}

function NativeWallet({
  initialShowHistory = false,
  preview = false,
}: WalletProps) {
  const { t } = useTranslation()
  const account = useAuthStore((state) => state.auth.user)
  const user = preview ? null : (account as UserWalletData | null)
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
          <div className='flex flex-wrap justify-end gap-2'>
            <Button
              variant='outline'
              disabled={preview}
              onClick={() => setHistoryOpen(true)}
            >
              <History className='size-4' aria-hidden='true' />
              {t('Order History')}
            </Button>
            <Button
              variant='outline'
              disabled={preview || refreshing}
              onClick={() => void refresh()}
            >
              <RefreshCw
                className={
                  refreshing ? 'size-4 motion-safe:animate-spin' : 'size-4'
                }
                aria-hidden='true'
              />
              {t('Refresh account status')}
            </Button>
          </div>
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='wallet-page mx-auto flex w-full max-w-6xl flex-col gap-8 pb-8 sm:gap-12'>
            {refreshFailed ? <p role='alert'>{t('Refresh failed')}</p> : null}
            {(preview || !isConsoleActivated(account)) && (
              <L0UpgradePanel preview={preview} />
            )}
            <div className='wallet-overview-grid grid items-start gap-8 xl:gap-10'>
              <WalletStatsCard
                user={user}
                balanceAction={
                  <Button variant='outline' disabled>
                    {t('Transfer')}
                  </Button>
                }
              />
              <div id='wallet-checkout' className='min-w-0 scroll-mt-4'>
                {nativeCreditsAvailable ? (
                  <NativeWalletCheckout />
                ) : (
                  <section className='wallet-checkout bg-card space-y-6 rounded-2xl p-6 sm:p-8'>
                    <h2 className='text-xl font-semibold tracking-tight'>
                      {t('Add funds')}
                    </h2>
                    <p className='text-muted-foreground text-sm leading-7'>
                      {t(
                        'Wallet data is unavailable. Payments and transfers are disabled until it is restored.'
                      )}
                    </p>
                    <div className='flex flex-wrap gap-2'>
                      <Button disabled>{t('Add funds')}</Button>
                      <Button variant='outline' disabled>
                        {t('Subscriptions')}
                      </Button>
                    </div>
                  </section>
                )}
              </div>
            </div>
            {nativeCreditsAvailable ? (
              <div className='wallet-secondary-grid grid items-start gap-8 xl:gap-10'>
                <NativeWalletSubscriptions onUpdated={() => void refresh()} />
                <NativeWalletPromotions onUpdated={() => void refresh()} />
              </div>
            ) : null}
            <AffiliateRewardsCard user={user} />
          </div>
        </SectionPageLayout.Content>
      </SectionPageLayout>
      {!preview && (
        <BillingHistoryDialog
          open={historyOpen}
          onOpenChange={setHistoryOpen}
        />
      )}
    </>
  )
}
