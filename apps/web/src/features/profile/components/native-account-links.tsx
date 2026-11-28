/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { Link2, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Button } from '@/components/ui/button'
import type { SystemStatus } from '@/features/auth/types'

import {
  assertAccountLinkScope,
  type FixedAccountProvider,
} from '../account-links-api'
import { useAccountBindingFlow } from '../hooks/use-account-binding-flow'
import { BindingCard } from './binding-card'
import { TelegramBindDialog } from './dialogs/telegram-bind-dialog'
import { WeChatBindDialog } from './dialogs/wechat-bind-dialog'

const labels: Record<FixedAccountProvider, string> = {
  google: 'Google',
  github: 'GitHub',
  linuxdo: 'LinuxDO',
  discord: 'Discord',
}
const TELEGRAM = 'https://telegram.org'
export function NativeAccountLinks({
  status,
  onUpdate,
}: {
  status: SystemStatus
  onUpdate: () => void | Promise<void>
}) {
  const { t } = useTranslation()
  const flow = useAccountBindingFlow(status, onUpdate)
  const { view, failed, target } = flow.state
  const { busy, codeOperation } = flow
  const ownerKey = JSON.stringify([flow.state.owner.user, flow.state.owner.sid])
  const [telegram, setTelegram] = useState({ ownerKey, open: false })
  if (telegram.ownerKey !== ownerKey) setTelegram({ ownerKey, open: false })
  const telegramBotName = (status.telegram_bot_name ?? '')
    .trim()
    .replace(/^@/, '')
    .trim()
  const canBindTelegram =
    status.telegram_oauth === true && Boolean(telegramBotName)
  const refreshTelegram = async () => {
    const scope = flow.state.viewScope
    if (!scope) throw new axios.CanceledError('Binding superseded')
    assertAccountLinkScope(scope)
    const outcome = await flow.load(TELEGRAM)
    if (outcome === 'failed') throw new Error('Failed to load profile')
    if (outcome !== 'loaded') {
      throw new axios.CanceledError('Binding superseded')
    }
    assertAccountLinkScope(scope)
    await onUpdate()
  }
  const rows: Array<{
    issuer: string
    label: string
    canBind: boolean
    onBind?: () => void
  }> = view
    ? view.providers.map((provider) => ({
        issuer: provider.issuer,
        label:
          'native_federated' in provider
            ? provider.name
            : labels[provider.provider],
        canBind: provider.can_bind,
        onBind: () => void flow.begin(provider),
      }))
    : []
  if (
    view &&
    (status.telegram_oauth === true ||
      view.links.some((link) => link.issuer === TELEGRAM && link.active))
  ) {
    rows.push({
      issuer: TELEGRAM,
      label: 'Telegram',
      canBind: canBindTelegram,
      onBind: () => setTelegram({ ownerKey, open: true }),
    })
  }
  if (view) {
    for (const link of view.links) {
      if (link.active && !rows.some((row) => row.issuer === link.issuer)) {
        rows.push({
          issuer: link.issuer,
          label: 'provider_id' in link ? link.name : link.issuer,
          canBind: false,
        })
      }
    }
  }
  const hasProvider =
    rows.some((row) => row.canBind) || view?.links.some((link) => link.active)
  return (
    <section
      className='mb-4 space-y-3'
      aria-label={t('Linked sign-in accounts')}
    >
      <div className='flex items-start justify-between gap-3'>
        <div>
          <h3 className='text-sm font-medium'>
            {t('Linked sign-in accounts')}
          </h3>
          <p className='text-muted-foreground text-xs'>
            {t(
              'Account security requires a sign-in within the last 10 minutes. Keep another sign-in method before unlinking.'
            )}
          </p>
        </div>
        <Button
          size='sm'
          variant='outline'
          disabled={busy}
          onClick={() => void flow.load()}
          aria-label={t('Refresh')}
        >
          <RefreshCw className='size-4' />
        </Button>
      </div>
      {failed && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Account bindings are unavailable')}
        </p>
      )}
      {!view && !failed && (
        <p className='text-muted-foreground text-sm'>{t('Loading...')}</p>
      )}
      {view && !hasProvider && (
        <p className='text-muted-foreground text-sm'>
          {t('No providers available')}
        </p>
      )}
      <div className='grid gap-3 sm:grid-cols-2'>
        {rows.map((row) => {
          const link = view?.links.find(
            (link) => link.active && link.issuer === row.issuer
          )
          if (!link && !row.canBind) return null
          return (
            <BindingCard
              key={row.issuer}
              label={row.label}
              value={link?.subject}
              icon={<Link2 className='size-4 shrink-0' />}
              bound={Boolean(link)}
              showBadge={false}
              action={{
                label: link ? t('Unbind') : t('Bind'),
                disabled: busy,
                destructive: Boolean(link),
                onClick: () =>
                  link ? flow.selectTarget(link) : row.onBind?.(),
              }}
            />
          )
        })}
      </div>
      {canBindTelegram && (
        <TelegramBindDialog
          open={telegram.ownerKey === ownerKey && telegram.open}
          onOpenChange={(open) =>
            setTelegram((current) =>
              current.ownerKey === ownerKey ? { ownerKey, open } : current
            )
          }
          botName={telegramBotName}
          onSuccess={refreshTelegram}
        />
      )}
      <WeChatBindDialog
        key={codeOperation?.state ?? 'closed'}
        open={codeOperation !== null}
        qrCodeUrl={
          codeOperation && 'native_federated' in codeOperation.descriptor
            ? codeOperation.descriptor.qr_code_url
            : ''
        }
        onOpenChange={flow.onCodeDialogChange}
        onSubmitCode={flow.submitCode}
      />
      <ConfirmDialog
        open={target !== null}
        onOpenChange={(open) => {
          if (!open && !busy) flow.selectTarget(null)
        }}
        title={t('Unbind account')}
        desc={t(
          'This account will no longer sign you in. Other sessions will be signed out and identity history is retained.'
        )}
        confirmText={t('Unbind')}
        destructive
        isLoading={busy}
        handleConfirm={() => void flow.unlink()}
      />
    </section>
  )
}
