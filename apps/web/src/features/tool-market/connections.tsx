/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { useAuthStore } from '@/stores/auth-store'

import {
  MarketAPIError,
  marketAPI,
  marketSupports,
  type Budget,
  type Grant,
  type Installation,
  type MarketConfig,
  type MarketToken,
} from './api'
import {
  marketConnectionNamespace,
  registerMarketConnectionTranslations,
  type MarketConnectionCopyKey,
} from './connection-i18n'
import {
  connectionStatus,
  isPersonalMarketClient,
  marketEndpoint,
} from './connection-utils'
import { MetaDelegationSettings } from './meta-delegation'
import { metaDelegationAPI } from './meta-delegation-api'
import { useMarketMoneyDraft } from './money'
import { MarketOAuthConnection } from './oauth-connection'
import { useMarketTranslation as useTranslation } from './provider-i18n'

type ClientAccess = {
  tokens: MarketToken[]
  grants: Grant[]
  installations: Installation[]
}

// Remount all local state on account changes; a one-time secret must never survive
// logout/login or become visible to the next account in the same browser session.
export function MarketConnections({
  config,
  onChooseClient,
}: {
  config: MarketConfig
  onChooseClient?: (clientID: string) => void
}) {
  const userID = useAuthStore((state) => state.auth.user?.id)
  return userID ? (
    <ConnectionWorkspace
      key={userID}
      userID={userID}
      config={config}
      onChooseClient={onChooseClient}
    />
  ) : null
}

function ConnectionWorkspace({
  userID,
  config,
  onChooseClient,
}: {
  userID: number
  config: MarketConfig
  onChooseClient?: (clientID: string) => void
}) {
  const { t, i18n } = useTranslation()
  const { formatQuota: formatRawQuota, label, step } = useWalletCurrency()
  const formatQuota = (quota: number) =>
    formatRawQuota(quota, { digitsLarge: 8, digitsSmall: 8 })
  registerMarketConnectionTranslations(i18n)
  const m = (
    key: MarketConnectionCopyKey,
    values?: Record<string, string | number>
  ) => String(t(key, { ...values, ns: marketConnectionNamespace }))
  const cache = useQueryClient()
  const metaSupported = marketSupports(config, 'meta_delegation')
  const oauthEnabled = marketSupports(config, 'mcp_oauth')
  const metaOAuthEnabled = metaSupported && oauthEnabled
  const cleanupSupported = marketSupports(config, 'client_record_cleanup')
  const tokens = useQuery({
    queryKey: ['tool-market', userID, 'tokens'],
    queryFn: ({ signal }) => marketAPI.mine<MarketToken>('tokens', signal),
  })
  const grants = useQuery({
    queryKey: ['tool-market', userID, 'grants'],
    queryFn: ({ signal }) => marketAPI.mine<Grant>('grants', signal),
  })
  const installations = useQuery({
    queryKey: ['tool-market', userID, 'installations'],
    queryFn: ({ signal }) =>
      marketAPI.mine<Installation>('installations', signal),
  })
  const budgets = useQuery({
    queryKey: ['tool-market', userID, 'budgets'],
    queryFn: ({ signal }) => marketAPI.mine<Budget>('budgets', signal),
  })
  const metaOAuthClients = useQuery({
    queryKey: ['tool-market', userID, 'meta-oauth-clients'],
    queryFn: ({ signal }) => metaDelegationAPI.oauthClients(signal),
    enabled: metaOAuthEnabled,
  })
  const [disconnect, setDisconnect] = useState<string | null>(null)
  const [scope, setScope] = useState('account')
  const [scopeID, setScopeID] = useState('')
  const limit = useMarketMoneyDraft(0)
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30000)
    return () => window.clearInterval(timer)
  }, [])
  const action = useMutation({
    retry: false,
    mutationFn: (operation: () => Promise<void>) => operation(),
    onSuccess: async () => {
      await cache.invalidateQueries({ queryKey: ['tool-market', userID] })
    },
  })
  let endpoint = ''
  try {
    endpoint = marketEndpoint(window.location.origin, config.mcp_path)
  } catch {
    // No token may be copied into an invalid or cross-origin configuration.
  }
  const budgetQuota = limit.quota
  const readError =
    tokens.isError ||
    grants.isError ||
    installations.isError ||
    budgets.isError ||
    (metaOAuthEnabled && metaOAuthClients.isError)
  const accessReady =
    tokens.isSuccess && grants.isSuccess && installations.isSuccess
  const savedGroups = useMemo(() => {
    const rows = new Map<string, ClientAccess>()
    const ensure = (id: string) => {
      let row = rows.get(id)
      if (!row) {
        row = { tokens: [], grants: [], installations: [] }
        rows.set(id, row)
      }
      return row
    }
    for (const row of tokens.data ?? []) ensure(row.client_id).tokens.push(row)
    for (const row of grants.data ?? []) ensure(row.client_id).grants.push(row)
    for (const row of installations.data ?? []) {
      ensure(row.client_id).installations.push(row)
    }
    if (metaOAuthEnabled) {
      for (const row of metaOAuthClients.data ?? []) ensure(row.client_id)
    }
    return [...rows].sort(([a], [b]) => a.localeCompare(b))
  }, [
    tokens.data,
    grants.data,
    installations.data,
    metaOAuthClients.data,
    metaOAuthEnabled,
  ])
  const groups = savedGroups
    .map(([id, group]): [string, ClientAccess] => [
      id,
      {
        tokens: group.tokens.filter((row) => !row.revoked_at),
        grants: group.grants.filter((row) => !row.revoked_at),
        installations: group.installations,
      },
    ])
    .filter(
      ([id, group]) =>
        group.tokens.length + group.grants.length + group.installations.length >
          0 ||
        (metaOAuthEnabled &&
          metaOAuthClients.data?.some((row) => row.client_id === id))
    )
  const revokedGroups = savedGroups
    .map(([id, group]): [string, ClientAccess] => [
      id,
      {
        tokens: group.tokens.filter((row) => !!row.revoked_at),
        grants: group.grants.filter((row) => !!row.revoked_at),
        installations: [],
      },
    ])
    .filter(([, group]) => group.tokens.length + group.grants.length > 0)
  const errorKeys: Record<string, MarketConnectionCopyKey> = {
    TOOL_MARKET_BUDGET: 'budgetError',
    TOOL_MARKET_BALANCE: 'balanceError',
    TOOL_MARKET_DENIED: 'deniedError',
    TOOL_MARKET_NOT_FOUND: 'deniedError',
    TOOL_MARKET_BUSY: 'busyError',
    TOOL_MARKET_CONFLICT: 'conflictError',
    TOOL_MARKET_REMOTE_CHANGED: 'conflictError',
  }
  const actionError =
    action.error instanceof MarketAPIError
      ? (errorKeys[action.error.code] ?? 'operationFailed')
      : 'operationFailed'
  const editBudget = (budget: Budget) => {
    setScope(budget.scope)
    setScopeID(budget.scope_id)
    limit.setQuota(budget.limit_quota)
    action.reset()
    document.getElementById('budget-limit')?.focus()
  }

  const renderClientGroups = (rows: typeof groups, history = false) =>
    rows.map(([id, group]) => {
      const saved =
        savedGroups.find(([clientID]) => clientID === id)?.[1] ?? group
      const allRevoked =
        saved.tokens.every((row) => !!row.revoked_at) &&
        saved.grants.every((row) => !!row.revoked_at)
      const hasAccess =
        group.tokens.some((row) => !row.revoked_at) ||
        group.grants.some((row) => !row.revoked_at) ||
        group.installations.length > 0
      return (
        <article
          key={id}
          className='bg-card min-w-0 space-y-4 rounded-xl border p-4 sm:p-5'
        >
          <div className='flex flex-wrap items-start justify-between gap-3'>
            <h4 className='min-w-0 flex-1 basis-40 font-semibold break-all'>
              {id}
            </h4>
            {onChooseClient && (
              <Button variant='outline' onClick={() => onChooseClient(id)}>
                {history ? t('Set up again') : m('chooseTools')}
              </Button>
            )}
            {!history && isPersonalMarketClient(id) && hasAccess && (
              <Button
                variant='outline'
                disabled={action.isPending || !accessReady}
                onClick={() => {
                  action.reset()
                  setDisconnect(id)
                }}
              >
                {m('disconnect')}
              </Button>
            )}
            {cleanupSupported && isPersonalMarketClient(id) && allRevoked && (
              <Button
                variant='ghost'
                disabled={action.isPending || !accessReady}
                onClick={() =>
                  action.mutate(async () => {
                    await marketAPI.removeClient(id)
                  })
                }
              >
                {t('Delete client')}
              </Button>
            )}
          </div>
          {metaOAuthEnabled &&
            !history &&
            metaOAuthClients.data?.some((row) => row.client_id === id) && (
              <MetaDelegationSettings
                target={{ kind: 'oauth', id }}
                permitted
              />
            )}
          {group.tokens.map((token) => (
            <div
              key={token.id}
              className='flex flex-wrap items-center justify-between gap-3 border-t pt-3 text-sm'
            >
              <div className='min-w-0 space-y-2'>
                <div className='flex flex-wrap gap-2'>
                  <Badge
                    variant={
                      connectionStatus(token, now) === 'active'
                        ? 'secondary'
                        : 'outline'
                    }
                  >
                    {m(connectionStatus(token, now))}
                  </Badge>
                  {token.can_invoke && (
                    <Badge variant='outline'>{m('invoke')}</Badge>
                  )}
                  {token.can_manage && (
                    <Badge variant='outline'>{m('manage')}</Badge>
                  )}
                  {!token.can_invoke && !token.can_manage && (
                    <Badge variant='outline'>{m('readOnly')}</Badge>
                  )}
                </div>
                <time
                  className='text-muted-foreground text-xs'
                  dateTime={new Date(token.expires_at * 1000).toISOString()}
                >
                  {new Date(token.expires_at * 1000).toLocaleString()}
                </time>
              </div>
              {metaSupported &&
                connectionStatus(token, now) === 'active' &&
                token.can_invoke &&
                token.can_manage && (
                  <MetaDelegationSettings
                    target={{ kind: 'personal', id: token.id }}
                    permitted
                  />
                )}
              {(!token.revoked_at || cleanupSupported) && (
                <Button
                  variant='ghost'
                  disabled={action.isPending || !accessReady}
                  onClick={() =>
                    action.mutate(async () => {
                      if (token.revoked_at) {
                        await marketAPI.removeTokenRecord(token.id)
                      } else {
                        await marketAPI.revokeToken(token.id)
                      }
                    })
                  }
                >
                  {token.revoked_at
                    ? t('Delete connection token')
                    : t('Revoke')}
                </Button>
              )}
            </div>
          ))}
          {(group.installations.length > 0 || group.grants.length > 0) && (
            <details className='border-t pt-3 text-sm'>
              <summary className='focus-visible:ring-ring cursor-pointer rounded-sm py-2 font-medium outline-none focus-visible:ring-2'>
                {t('Loaded tools and authorizations')}
              </summary>
              <div className='mt-3 space-y-4'>
                {group.installations.map((item) => (
                  <div
                    key={`${item.tool_id}:${item.version_id}`}
                    className='flex flex-wrap items-center justify-between gap-3'
                  >
                    <code className='text-muted-foreground min-w-0 text-xs break-all'>
                      {item.tool_id}
                    </code>
                    <Button
                      variant='outline'
                      disabled={action.isPending || !accessReady}
                      onClick={() =>
                        action.mutate(async () => {
                          await marketAPI.install(item, false)
                        })
                      }
                    >
                      {t('Unload')}
                    </Button>
                  </div>
                ))}
                {group.grants.map((grant) => (
                  <div key={grant.id} className='space-y-2 border-t pt-3'>
                    <code className='text-muted-foreground block text-xs break-all'>
                      {grant.tool_id}
                    </code>
                    <div className='flex flex-wrap items-center justify-between gap-3'>
                      <div className='space-y-1'>
                        <Badge variant='outline'>
                          {m(connectionStatus(grant, now))}
                        </Badge>
                        <p className='tabular-nums'>
                          {formatQuota(
                            grant.spent_quota + grant.reserved_quota
                          )}{' '}
                          / {formatQuota(grant.max_total_quota)}
                        </p>
                        <p className='text-muted-foreground text-xs'>
                          {t('Remaining successful calls')}:{' '}
                          {Math.max(
                            0,
                            grant.max_calls -
                              grant.successful_calls -
                              grant.reserved_calls
                          )}
                        </p>
                      </div>
                      {(!grant.revoked_at || cleanupSupported) && (
                        <Button
                          variant='ghost'
                          disabled={action.isPending || !accessReady}
                          onClick={() =>
                            action.mutate(async () => {
                              if (grant.revoked_at) {
                                await marketAPI.removeGrantRecord(grant.id)
                              } else {
                                await marketAPI.revokeGrant(grant.id)
                              }
                            })
                          }
                        >
                          {grant.revoked_at
                            ? t('Delete authorization')
                            : t('Revoke authorization')}
                        </Button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </details>
          )}
        </article>
      )
    })

  return (
    <div className='space-y-8'>
      {(action.isError || readError) && (
        <div
          role='alert'
          className='border-destructive/30 bg-destructive/5 flex flex-wrap items-center justify-between gap-3 rounded-xl border p-4 text-sm'
        >
          <p>{m(action.isError ? actionError : 'operationFailed')}</p>
          {readError && (
            <Button
              variant='outline'
              onClick={() =>
                void Promise.allSettled([
                  tokens.refetch(),
                  grants.refetch(),
                  installations.refetch(),
                  budgets.refetch(),
                  ...(metaOAuthEnabled ? [metaOAuthClients.refetch()] : []),
                ])
              }
            >
              {t('Retry')}
            </Button>
          )}
        </div>
      )}
      {action.isSuccess && (
        <p role='status' className='text-muted-foreground text-sm'>
          {m('saved')}
        </p>
      )}
      <div className='grid items-start gap-8 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]'>
        <section className='bg-card min-w-0 space-y-5 rounded-xl border p-4 sm:p-6'>
          <div className='space-y-2'>
            <h3 className='text-lg font-semibold'>
              {t('Connect your MCP client')}
            </h3>
          </div>
          {oauthEnabled && endpoint ? (
            <MarketOAuthConnection endpoint={endpoint} />
          ) : (
            <p role='status' className='text-muted-foreground text-sm'>
              {t('Unavailable')}
            </p>
          )}
        </section>

        <section className='min-w-0 space-y-4'>
          <div className='flex items-center justify-between gap-3'>
            <h3 className='text-lg font-semibold'>{m('clients')}</h3>
            {accessReady && <Badge variant='outline'>{groups.length}</Badge>}
          </div>
          {(tokens.isPending ||
            grants.isPending ||
            installations.isPending) && (
            <p role='status' className='text-muted-foreground py-6 text-sm'>
              {t('Loading…')}
            </p>
          )}
          {accessReady && !groups.length && (
            <div className='rounded-xl border border-dashed px-6 py-12 text-center'>
              <h4 className='font-medium'>{m('empty')}</h4>
              <p className='text-muted-foreground mx-auto mt-2 max-w-sm text-sm'>
                {t(
                  'Load a tool from the market, then authorize its client and spending limits.'
                )}
              </p>
            </div>
          )}
          <div data-testid='market-client-list' className='space-y-4'>
            {renderClientGroups(groups)}
          </div>
          {accessReady && revokedGroups.length > 0 && (
            <details
              data-testid='market-revoked-records'
              className='space-y-4 border-t pt-3'
            >
              <summary className='focus-visible:ring-ring cursor-pointer rounded-sm py-2 text-sm font-medium outline-none focus-visible:ring-2'>
                {t('Revoked records')}
              </summary>
              {renderClientGroups(revokedGroups, true)}
            </details>
          )}
        </section>
      </div>

      <section className='min-w-0 space-y-5 border-t pt-6'>
        <div className='space-y-2'>
          <h3 className='text-lg font-semibold'>{t('Spending budgets')}</h3>
          <p className='text-muted-foreground max-w-[70ch] text-sm leading-6'>
            {t(
              'Budgets include reserved and spent credits. These are cumulative limits; changing them does not reset usage.'
            )}
          </p>
        </div>
        <form
          className='grid items-end gap-4 sm:grid-cols-2 xl:grid-cols-4 [&_input]:min-h-11'
          onSubmit={(event) => {
            event.preventDefault()
            if (
              budgetQuota === undefined ||
              !budgets.isSuccess ||
              action.isPending
            ) {
              return
            }
            const amount = budgetQuota
            action.mutate(async () => {
              await marketAPI.budget({
                scope,
                scope_id: scope === 'account' ? '' : scopeID.trim(),
                limit_quota: amount,
              })
            })
          }}
        >
          <Field>
            <FieldLabel htmlFor='budget-scope'>{t('Budget scope')}</FieldLabel>
            <select
              id='budget-scope'
              className='border-input bg-background focus-visible:ring-ring min-h-11 w-full rounded-md border px-3 text-base outline-none focus-visible:ring-2 sm:text-sm'
              value={scope}
              onChange={(event) => {
                setScope(event.target.value)
                setScopeID('')
              }}
            >
              <option value='account'>{t('Account')}</option>
              <option value='client'>{t('Client')}</option>
              <option value='tool'>{t('Tool')}</option>
            </select>
          </Field>
          {scope !== 'account' && (
            <Field>
              <FieldLabel htmlFor='budget-id'>
                {scope === 'client' ? t('Client ID') : t('Tool ID')}
              </FieldLabel>
              <Input
                id='budget-id'
                required
                value={scopeID}
                maxLength={128}
                onChange={(event) => setScopeID(event.target.value)}
              />
            </Field>
          )}
          <Field>
            <FieldLabel htmlFor='budget-limit'>
              {t('Total spending limit')} ({label})
            </FieldLabel>
            <Input
              id='budget-limit'
              inputMode='decimal'
              min={0}
              step={step}
              required
              value={limit.input}
              aria-invalid={budgetQuota === undefined}
              onChange={(event) => limit.setInput(event.target.value)}
            />
          </Field>
          <Button
            type='submit'
            className='min-h-11'
            disabled={
              action.isPending ||
              !budgets.isSuccess ||
              budgetQuota === undefined ||
              (scope !== 'account' && !scopeID.trim())
            }
          >
            {t('Save budget')}
          </Button>
        </form>
        <p className='text-muted-foreground text-xs'>
          {budgetQuota === undefined ? m('invalidBudget') : m('zeroBudget')}
        </p>
        {budgets.isPending && (
          <p role='status' className='text-muted-foreground text-sm'>
            {t('Loading…')}
          </p>
        )}
        <div className='grid gap-4 md:grid-cols-2 xl:grid-cols-3'>
          {(budgets.data ?? []).map((budget) => {
            const used = budget.spent_quota + budget.reserved_quota
            return (
              <div
                key={`${budget.scope}:${budget.scope_id}`}
                className='bg-muted/40 min-w-0 space-y-3 rounded-lg p-4 text-sm'
              >
                <div className='flex items-start justify-between gap-3'>
                  <p className='min-w-0 font-medium break-all'>
                    {budget.scope === 'account'
                      ? t('Account')
                      : budget.scope === 'client'
                        ? t('Client')
                        : t('Tool')}
                    {budget.scope_id && ` · ${budget.scope_id}`}
                  </p>
                  <Button
                    variant='ghost'
                    disabled={action.isPending || !budgets.isSuccess}
                    onClick={() => editBudget(budget)}
                  >
                    {m('editBudget')}
                  </Button>
                </div>
                <p className='tabular-nums'>
                  {formatQuota(used)} / {formatQuota(budget.limit_quota)}
                </p>
                <progress
                  aria-label={t('Total spending limit')}
                  className='accent-foreground h-1.5 w-full'
                  value={Math.min(used, Math.max(1, budget.limit_quota))}
                  max={Math.max(1, budget.limit_quota)}
                />
              </div>
            )
          })}
        </div>
      </section>

      <Dialog
        open={disconnect !== null}
        onOpenChange={(open) => {
          if (!open && !action.isPending) setDisconnect(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {m('disconnectTitle', { client: disconnect ?? '' })}
            </DialogTitle>
            <DialogDescription>{m('disconnectWarning')}</DialogDescription>
          </DialogHeader>
          {action.isError && (
            <p role='alert' className='text-destructive text-sm'>
              {m(actionError)}
            </p>
          )}
          <div className='flex justify-end gap-2'>
            <Button
              variant='outline'
              disabled={action.isPending}
              onClick={() => setDisconnect(null)}
            >
              {t('Cancel')}
            </Button>
            <Button
              variant='destructive'
              disabled={action.isPending || !disconnect || !accessReady}
              onClick={() => {
                if (!disconnect) return
                const clientID = disconnect
                action.mutate(async () => {
                  await marketAPI.disconnectClient(clientID)
                  setDisconnect(null)
                })
              }}
            >
              {m('disconnect')}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
