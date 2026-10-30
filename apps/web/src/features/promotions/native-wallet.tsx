/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  commerceScope,
  CommerceFailure,
  currentCommerceScope,
  formatCredits,
  nativeToken,
  operationKey,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { useAuthStore } from '@/stores/auth-store'

import {
  campaignSchema,
  claimCampaign,
  listCampaigns,
  redeemCode,
  redemptionDigest,
  rewardHistory,
  type NativeCampaign,
  type NativeReward,
} from './native-api'

const pendingSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('redemption'),
      operation_key: operationKey,
      digest: nativeToken,
    })
    .strict(),
  z
    .object({
      kind: z.literal('campaign'),
      operation_key: operationKey,
      campaign: campaignSchema,
    })
    .strict(),
])
type Pending = z.infer<typeof pendingSchema>
export function NativeWalletPromotions({
  onUpdated,
}: {
  onUpdated?: () => void
}) {
  const auth = useAuthStore((state) => state.auth)
  const scope = commerceScope()
  return scope ? (
    <Rewards
      key={`${auth.user?.id}:${auth.session?.sid}`}
      scope={scope}
      onUpdated={onUpdated}
    />
  ) : null
}
function Rewards({
  scope,
  onUpdated,
}: {
  scope: CommerceScope
  onUpdated?: () => void
}) {
  const { t, i18n } = useTranslation()
  const [campaigns, setCampaigns] = useState<NativeCampaign[]>([])
  const [history, setHistory] = useState<NativeReward[]>([])
  const [pending, setPending] = useState<Pending | null>(null)
  const [code, setCode] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const [blocked, setBlocked] = useState(false)
  const [refused, setRefused] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const current = () => mounted.current && currentCommerceScope(scope)
  async function load() {
    try {
      const [campaigns, history] = await Promise.all([
        listCampaigns(scope),
        rewardHistory(scope),
      ])
      if (current()) {
        setCampaigns(campaigns)
        setHistory(history)
      }
    } catch {
      if (current()) setNotice(t('Request failed'))
    }
  }
  useEffect(() => {
    mounted.current = true
    try {
      setPending(readPending(scope, 'rewards', pendingSchema))
    } catch {
      setBlocked(true)
    }
    void load()
    return () => {
      mounted.current = false
    }
    // Account/session changes remount the isolated draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  function remember(value: Pending | null) {
    try {
      savePending(scope, 'rewards', value)
      setPending(value)
      return true
    } catch {
      setBlocked(true)
      return false
    }
  }
  async function execute(next: Pending) {
    if (running.current || blocked || !current()) return
    running.current = true
    setBusy(true)
    setNotice('')
    setRefused(false)
    try {
      if (
        next.kind === 'redemption' &&
        next.digest !== (await redemptionDigest(code))
      ) {
        setNotice(t('Re-enter the same code to recover this request.'))
        return
      }
      if (!remember(next)) return
      const result =
        next.kind === 'redemption'
          ? await redeemCode(scope, next.operation_key, code)
          : await claimCampaign(scope, next.campaign, next.operation_key)
      if (current()) {
        remember(null)
        setCode('')
        setNotice(
          `${t('Reward received')} · ${formatCredits(result.amount_units, i18n.language)} ${t('Core credits')} · #${result.journal_id}`
        )
        onUpdated?.()
        await load()
      }
    } catch (error) {
      if (current()) {
        const denied = error instanceof CommerceFailure && !error.uncertain
        setRefused(denied)
        setNotice(
          t(
            denied
              ? 'Request rejected. Refresh and review before trying again.'
              : 'Result unknown. Retry the saved request before starting another.'
          )
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (pending) {
      await execute(pending)
      return
    }
    if (busy || blocked) return
    const digest = await redemptionDigest(code)
    if (current()) {
      await execute({ kind: 'redemption', operation_key: requestKey(), digest })
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t('Rewards')}</CardTitle>
        <CardDescription>
          {t(
            'Daily check-in resets at midnight UTC. Campaign rewards use a fixed budget.'
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className='space-y-4'>
        {notice && (
          <Alert role='status'>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        )}
        {blocked && (
          <Alert role='alert'>
            <AlertDescription>
              {t(
                'Recovery storage is unavailable. No new request can be sent.'
              )}
            </AlertDescription>
          </Alert>
        )}
        <form onSubmit={(event) => void submit(event)} className='space-y-2'>
          <Label htmlFor='native-redeem-code'>{t('Redemption Code')}</Label>
          <div className='flex flex-wrap gap-2'>
            <Input
              id='native-redeem-code'
              type='password'
              autoComplete='off'
              spellCheck={false}
              className='min-w-0 flex-1'
              value={code}
              disabled={busy || pending?.kind === 'campaign'}
              onChange={(event) => setCode(event.target.value)}
              required
              maxLength={48}
            />
            <Button
              type='submit'
              disabled={
                busy || blocked || !code || pending?.kind === 'campaign'
              }
            >
              {t(pending ? 'Retry saved request' : 'Redeem')}
            </Button>
          </div>
        </form>
        {pending?.kind === 'redemption' && (
          <p className='text-muted-foreground text-sm'>
            {t('Re-enter the same code to recover this request.')}
          </p>
        )}
        {pending?.kind === 'campaign' && (
          <Button
            disabled={busy || blocked}
            onClick={() => void execute(pending)}
          >
            {t('Retry saved request')} · {pending.campaign.spec.title}
          </Button>
        )}
        {pending && refused && (
          <Button
            variant='outline'
            disabled={busy}
            onClick={() => {
              remember(null)
              setNotice('')
              setRefused(false)
            }}
          >
            {t('Close')}
          </Button>
        )}
        <ul className='divide-y'>
          {campaigns
            .filter((campaign) => campaign.spec.kind !== 'redemption')
            .map((campaign) => (
              <li
                key={campaign.id}
                className='flex flex-wrap items-center justify-between gap-3 py-3'
              >
                <div>
                  <p className='font-medium'>{campaign.spec.title}</p>
                  <p className='text-muted-foreground text-sm'>
                    {formatCredits(campaign.spec.award_units, i18n.language)}{' '}
                    {t('Core credits')}
                  </p>
                </div>
                <Button
                  disabled={busy || blocked || !!pending}
                  onClick={() =>
                    void execute({
                      kind: 'campaign',
                      campaign,
                      operation_key: requestKey(),
                    })
                  }
                >
                  {t(
                    campaign.spec.kind === 'checkin'
                      ? 'Check In'
                      : 'Claim reward'
                  )}
                </Button>
              </li>
            ))}
        </ul>
        <div className='flex items-center justify-between gap-2'>
          <h3 className='font-medium'>{t('Reward history')}</h3>
          <Button
            variant='outline'
            size='sm'
            disabled={busy}
            onClick={() => void load()}
          >
            {t('Refresh')}
          </Button>
        </div>
        <ul className='divide-y'>
          {history.slice(0, 20).map((receipt) => (
            <li
              key={receipt.id}
              className='flex flex-wrap justify-between gap-2 py-2 text-sm'
            >
              <span>
                {formatCredits(receipt.amount_units, i18n.language)}{' '}
                {t('Core credits')}
              </span>
              <span className='text-muted-foreground'>
                {new Date(receipt.claimed_at * 1000).toLocaleString()} · #
                {receipt.journal_id}
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}
