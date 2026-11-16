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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useLocation } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import {
  commerceScope,
  currentCommerceScope,
} from '@/features/commerce/native-client'
import { ForgePublicShell } from '@/features/forge/forge-public-shell'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import { claimTransfer, inspectTransfer } from './api'
import { WalletOutcomeUnknown } from './flow'

const pendingTransferKey = 'wallet-transfer-login'
export function ClaimTransfer() {
  const auth = useAuthStore((state) => state.auth)
  const location = useLocation()
  return (
    <ScopedClaimTransfer
      key={`${auth.user?.id}:${auth.session?.sid}:${location.hash}`}
    />
  )
}
function ScopedClaimTransfer() {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  const scope = commerceScope()
  const mounted = useRef(true)
  const current = () =>
    mounted.current && !!scope && currentCommerceScope(scope)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])
  const location = useLocation()
  const hash = location.hash.replace(/^#/, '')
  const [saved] = useState(() => {
    // A supplied fragment identifies this link even when malformed. Recovery
    // belongs only to the empty-fragment return from sign-in.
    if (hash) return { token: '', unavailable: false }
    try {
      return {
        token: sessionStorage.getItem(pendingTransferKey) ?? '',
        unavailable: false,
      }
    } catch {
      return { token: '', unavailable: true }
    }
  })
  const [recoveryUnavailable, setRecoveryUnavailable] = useState(
    saved.unavailable
  )
  const token = hash || saved.token
  const valid = /^[a-f0-9]{64}$/.test(token)
  const client = useQueryClient()
  const key = [
    'wallet-transfer-receipt',
    scope?.userId,
    scope?.sessionId,
    token,
  ]
  const receipt = useQuery({
    queryKey: key,
    queryFn: () => {
      if (!scope) throw new WalletOutcomeUnknown()
      return inspectTransfer(scope, token)
    },
    enabled: !!scope && valid,
    retry: false,
    refetchInterval: scope && valid ? 10000 : false,
  })
  const claim = useMutation({
    mutationFn: () => {
      if (!scope) throw new WalletOutcomeUnknown()
      return claimTransfer(scope, token)
    },
    onSuccess: (data) => {
      if (!current()) return
      client.setQueryData(key, data)
      try {
        sessionStorage.removeItem(pendingTransferKey)
      } catch {
        // A confirmed Core receipt remains successful if local cleanup is blocked.
      }
      toast.success(t('Transfer claimed. Your wallet has been credited.'))
    },
    onError: (err: Error) => {
      if (!current()) return
      toast.error(t(err.message))
      void receipt.refetch()
    },
  })
  const data = receipt.data
  return (
    <ForgePublicShell>
      <main className='mx-auto grid w-full max-w-lg gap-5 p-6'>
        <h1 className='text-2xl font-semibold'>{t('Wallet transfer')}</h1>
        <p className='text-destructive text-sm'>
          {t('Do not share this link publicly.')}
        </p>
        {recoveryUnavailable && (
          <p role='alert'>
            {t('Recovery storage is unavailable. No new request can be sent.')}
          </p>
        )}
        {!valid ? (
          !recoveryUnavailable && (
            <p role='alert'>{t('Invalid transfer link.')}</p>
          )
        ) : !scope ? (
          <>
            <p>{t('Sign in to claim this transfer.')}</p>
            <Button
              render={
                <Link
                  to='/sign-in'
                  search={{ redirect: '/transfer' }}
                  onClick={(event) => {
                    // Save before leaving; the bearer never belongs in a login URL.
                    try {
                      sessionStorage.setItem(pendingTransferKey, token)
                      setRecoveryUnavailable(false)
                    } catch {
                      event.preventDefault()
                      setRecoveryUnavailable(true)
                    }
                  }}
                />
              }
            >
              {t('Sign in')}
            </Button>
          </>
        ) : receipt.isPending ? (
          <p role='status'>{t('Loading...')}</p>
        ) : receipt.isError ? (
          <>
            <p role='alert'>{t('Unable to load this transfer.')}</p>
            <Button variant='outline' onClick={() => void receipt.refetch()}>
              {t('Retry')}
            </Button>
          </>
        ) : data ? (
          <>
            <p className='text-3xl font-semibold'>
              {data.quota.toLocaleString(locale)} {t('Core credits')}
            </p>
            <p>
              {t('Created at')}:{' '}
              {new Date(data.created_at * 1000).toLocaleString(locale)}
            </p>
            {data.status === 'pending' ? (
              data.is_sender ? (
                <p>{t('You cannot claim your own transfer.')}</p>
              ) : (
                <>
                  <Button
                    disabled={claim.isPending}
                    onClick={() => claim.mutate()}
                  >
                    {claim.isPending ? t('Claiming...') : t('Claim transfer')}
                  </Button>
                </>
              )
            ) : (
              <p role='status'>
                {data.status === 'cancelled'
                  ? t('This transfer was cancelled.')
                  : data.claimed_by_me
                    ? t('Transfer claimed. Your wallet has been credited.')
                    : t('This transfer has already been claimed.')}
              </p>
            )}
            {data.claimed_at > 0 && (
              <p>
                {t('Claimed at')}:{' '}
                {new Date(data.claimed_at * 1000).toLocaleString(locale)}
              </p>
            )}
            <Button variant='outline' render={<Link to='/wallet' />}>
              {t('Wallet')}
            </Button>
          </>
        ) : null}
      </main>
    </ForgePublicShell>
  )
}
