/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/

import { Key01Icon, ShieldUserIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  getOAuthGrantSummaries,
  revokeAllOAuthGrants,
  revokeOAuthGrant,
  revokeOAuthClient,
  type OAuthGrantSummary,
} from '../api'

function formatDate(value: number, language: string): string {
  return new Intl.DateTimeFormat(toIntlLocale(language), {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value * 1000))
}

function clientLabel(grant: OAuthGrantSummary, t: (value: string) => string) {
  return grant.purpose === 'normal_model'
    ? t('Model client')
    : t('Read-only client')
}

export function OAuthGrantsCard() {
  const { t, i18n } = useTranslation()
  const queryClient = useQueryClient()
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const grantsQueryKey = ['profile', 'oauth-grants', userId, sessionId] as const
  const [revokeTarget, setRevokeTarget] = useState<OAuthGrantSummary | null>(
    null
  )
  const [revokeAllOpen, setRevokeAllOpen] = useState(false)
  const grantsQuery = useQuery({
    queryKey: grantsQueryKey,
    queryFn: async () => {
      const response = await getOAuthGrantSummaries()
      if (!response.success) {
        throw new Error(
          response.message || t('Unable to load authorized clients')
        )
      }
      return response
    },
  })
  const revokeMutation = useMutation({
    mutationFn: revokeOAuthGrant,
    onSuccess: async (response) => {
      if (!response.success) {
        throw new Error(response.message || t('Unable to revoke access'))
      }
      setRevokeTarget(null)
      toast.success(t('Model access revoked'))
      await queryClient.invalidateQueries({ queryKey: grantsQueryKey })
    },
    onError: (error: Error) => toast.error(error.message),
  })
  const revokeAllMutation = useMutation({
    mutationFn: revokeAllOAuthGrants,
    onSuccess: async (response) => {
      if (!response.success) {
        throw new Error(response.message || t('Unable to revoke access'))
      }
      setRevokeAllOpen(false)
      toast.success(t('All model access was revoked'))
      await queryClient.invalidateQueries({ queryKey: grantsQueryKey })
    },
    onError: (error: Error) => toast.error(error.message),
  })

  const [clientTarget, setClientTarget] = useState<string | null>(null)
  const clientMutation = useMutation({
    mutationFn: revokeOAuthClient,
    onSuccess: async (response) => {
      if (!response.success) {
        throw new Error(response.message || t('Unable to revoke access'))
      }
      setClientTarget(null)
      toast.success(t('Client access revoked'))
      await queryClient.invalidateQueries({ queryKey: grantsQueryKey })
    },
    onError: (error: Error) => toast.error(error.message),
  })
  const busy =
    revokeMutation.isPending ||
    revokeAllMutation.isPending ||
    clientMutation.isPending
  const grants = grantsQuery.data?.data ?? []
  const active = grants.filter((grant) => grant.active)

  const clients = new Map<string, OAuthGrantSummary[]>()
  for (const grant of grants) {
    const entries = clients.get(grant.client_id) ?? []
    entries.push(grant)
    clients.set(grant.client_id, entries)
  }

  return (
    <>
      <Card data-card-hover='false'>
        <CardHeader className='has-data-[slot=card-action]:grid-cols-1 sm:has-data-[slot=card-action]:grid-cols-[1fr_auto]'>
          <CardTitle className='flex items-center gap-2'>
            <HugeiconsIcon icon={ShieldUserIcon} strokeWidth={2} />
            {t('Authorized clients')}
          </CardTitle>
          <CardDescription>
            {t(
              'Review applications that can read your account or call models. Revoking access takes effect immediately.'
            )}
          </CardDescription>
          <CardAction className='col-start-1 row-start-3 mt-2 justify-self-start sm:col-start-2 sm:row-start-1 sm:mt-0 sm:justify-self-end'>
            <Button
              type='button'
              variant='destructive'
              size='sm'
              disabled={active.length === 0 || busy || grantsQuery.isError}
              onClick={() => setRevokeAllOpen(true)}
            >
              {t('Revoke all access')}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent>
          {grantsQuery.isLoading ? (
            <div className='space-y-3'>
              <Skeleton className='h-24 w-full' />
              <Skeleton className='h-24 w-full' />
            </div>
          ) : grantsQuery.isError ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant='icon'>
                  <HugeiconsIcon icon={Key01Icon} strokeWidth={2} />
                </EmptyMedia>
                <EmptyTitle>
                  {t('Unable to load authorized clients')}
                </EmptyTitle>
                <EmptyDescription>
                  {t('Refresh the page and try again.')}
                </EmptyDescription>
              </EmptyHeader>
              <Button
                type='button'
                variant='outline'
                onClick={() => grantsQuery.refetch()}
              >
                {t('Retry')}
              </Button>
            </Empty>
          ) : grants.length === 0 ? (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant='icon'>
                  <HugeiconsIcon icon={Key01Icon} strokeWidth={2} />
                </EmptyMedia>
                <EmptyTitle>{t('No authorized clients')}</EmptyTitle>
                <EmptyDescription>
                  {t('Applications will appear here after you approve access.')}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <div className='space-y-3'>
              {[...clients].map(([clientId, clientGrants]) => (
                <section
                  key={clientId}
                  className='overflow-hidden rounded-xl border'
                  aria-label={clientId}
                >
                  <div className='bg-primary/5 flex flex-wrap items-center justify-between gap-3 border-b p-4'>
                    <div className='min-w-0'>
                      <h3 className='font-semibold break-all'>{clientId}</h3>
                      <p className='text-muted-foreground text-xs'>
                        {t('Active authorizations: {{count}}', {
                          count: clientGrants.filter((grant) => grant.active)
                            .length,
                        })}
                      </p>
                    </div>
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      disabled={
                        busy || !clientGrants.some((grant) => grant.active)
                      }
                      onClick={() => setClientTarget(clientId)}
                    >
                      {t('Revoke client access')}
                    </Button>
                  </div>
                  <div className='divide-y'>
                    {clientGrants.map((grant) => (
                      <div key={grant.id} className='p-4'>
                        <div className='flex flex-wrap items-start justify-between gap-3'>
                          <div className='min-w-0 flex-1 space-y-1 break-words'>
                            <p className='font-medium'>
                              {clientLabel(grant, t)}
                            </p>
                            <p className='text-muted-foreground text-sm'>
                              {grant.purpose === 'normal_model'
                                ? t('Model access')
                                : t('Read-only account access')}
                            </p>
                            {(grant.display_label ||
                              grant.managed_application) && (
                              <p className='text-muted-foreground text-xs'>
                                {grant.client_id === 'lmm-cc-switch'
                                  ? t('CC Switch')
                                  : (grant.managed_application ??
                                    grant.client_id)}
                                {grant.display_label
                                  ? ` · ${grant.display_label}`
                                  : ''}
                                {grant.installation_id
                                  ? ` (${grant.installation_id.slice(0, 8)})`
                                  : ''}
                              </p>
                            )}
                          </div>
                          {grant.active ? (
                            <Button
                              type='button'
                              variant='outline'
                              size='sm'
                              disabled={busy}
                              onClick={() => setRevokeTarget(grant)}
                            >
                              {t('Revoke')}
                            </Button>
                          ) : (
                            <Badge variant='secondary'>
                              {grant.revoked_at ? t('Revoked') : t('Expired')}
                            </Badge>
                          )}
                        </div>
                        <div className='mt-3 flex flex-wrap gap-2'>
                          {grant.scopes.map((scope) => (
                            <Badge key={scope} variant='outline'>
                              {scope}
                            </Badge>
                          ))}
                        </div>
                        <p className='text-muted-foreground mt-3 text-xs'>
                          {t('Created')}:{' '}
                          {formatDate(grant.created_at, i18n.language)}
                        </p>
                        <p className='text-muted-foreground mt-1 text-xs'>
                          {grant.budget_credits !== null
                            ? `${t('Budget')}: ${grant.budget_credits.toLocaleString(toIntlLocale(i18n.language))} ${t('credits')} · `
                            : ''}
                          {grant.active
                            ? `${t('Expires')}: ${formatDate(grant.expires_at, i18n.language)}`
                            : grant.revoked_at
                              ? `${t('Revoked')}: ${formatDate(grant.revoked_at, i18n.language)}`
                              : `${t('Expired')}: ${formatDate(grant.expires_at, i18n.language)}`}
                        </p>
                      </div>
                    ))}
                  </div>
                </section>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
      <ConfirmDialog
        open={clientTarget !== null}
        onOpenChange={(open) => !open && !busy && setClientTarget(null)}
        title={t('Revoke client access')}
        desc={t(
          'All authorizations for {{client}} will be revoked. Other clients keep their access.',
          { client: clientTarget ?? '' }
        )}
        destructive
        confirmText={t('Revoke access')}
        isLoading={clientMutation.isPending}
        handleConfirm={() => {
          if (clientTarget && !busy) clientMutation.mutate(clientTarget)
        }}
      />
      <ConfirmDialog
        open={revokeTarget !== null}
        onOpenChange={(open) => !open && setRevokeTarget(null)}
        title={t('Revoke model access?')}
        desc={t(
          'The client will stop receiving model authorization immediately. You can authorize it again later.'
        )}
        destructive
        confirmText={t('Revoke access')}
        isLoading={revokeMutation.isPending}
        handleConfirm={() => {
          if (revokeTarget) revokeMutation.mutate(revokeTarget.id)
        }}
      />
      <ConfirmDialog
        open={revokeAllOpen}
        onOpenChange={setRevokeAllOpen}
        title={t('Revoke all authorized clients?')}
        desc={t(
          'Every OAuth client shown here will lose access immediately, including model access.'
        )}
        destructive
        confirmText={t('Revoke all')}
        isLoading={revokeAllMutation.isPending}
        handleConfirm={() => revokeAllMutation.mutate()}
      />
    </>
  )
}
