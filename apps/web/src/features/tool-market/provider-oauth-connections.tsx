/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

import { useMutation, useQuery } from '@tanstack/react-query'
import { useMemo, useState } from 'react'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { useWalletCurrency } from '@/hooks/use-wallet-currency'
import { useAuthStore } from '@/stores/auth-store'

import { marketAPI, type OAuthProvider } from './api'
import { useMarketMoneyDraft } from './money'
import { useMarketTranslation } from './provider-i18n'
import {
  prepareRemoteAuthorization,
  remoteOAuthRedirectURI,
} from './remote-server-api'

export function ProviderOAuthConnections({
  available,
  redirectURI,
}: {
  available: boolean
  redirectURI?: string
}) {
  const { t } = useMarketTranslation()
  const [open, setOpen] = useState(false)
  const [selected, setSelected] = useState<{
    provider: OAuthProvider
    name: string
  }>()
  const price = useMarketMoneyDraft(Number.NaN)
  const { label } = useWalletCurrency()
  const [saved, setSaved] = useState(false)
  const userID = useAuthStore((s) => s.auth.user?.id)
  const sessionId = useAuthStore((s) => s.auth.session?.sid)
  const owner = useMemo(
    () => ({ userId: userID, sessionId }),
    [userID, sessionId]
  )
  const providers = useQuery({
    queryKey: ['tool-market', userID, 'oauth-providers'],
    queryFn: () => marketAPI.oauthProviders(owner),
    enabled: available && open && !!userID,
    retry: false,
  })
  const action = useMutation({
    mutationFn: (operation: () => Promise<void>) => operation(),
    retry: false,
    onSettled: () => {
      void providers.refetch()
    },
  })
  const { mutate } = action
  const names: Record<OAuthProvider, string> = {
    agentkey: 'AgentKey',
    treg: 'treg',
    'treg-catalog': t('treg — catalog only MCP'),
  }
  if (
    !available ||
    typeof window === 'undefined' ||
    !remoteOAuthRedirectURI(redirectURI)
  )
    return null
  return (
    <>
      <Button
        variant='outline'
        className='min-h-11'
        onClick={() => setOpen(true)}
      >
        {t('Connect a marketplace')}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('Connect a marketplace')}</DialogTitle>
          </DialogHeader>
          {providers.isFetching && !providers.data && (
            <p role='status'>{t('Loading')}</p>
          )}
          {(action.isError || providers.isError) && (
            <p role='alert'>{t('Connection failed')}</p>
          )}
          {providers.isError && (
            <Button variant='outline' onClick={() => void providers.refetch()}>
              {t('Retry')}
            </Button>
          )}
          {providers.data?.map(
            ({
              provider,
              connected,
              installation,
              tools,
              fixed_price_tools,
            }) => (
              <div
                key={provider}
                className='space-y-2 border-b py-3 last:border-b-0'
              >
                <div className='flex flex-wrap items-center justify-between gap-3'>
                  <span className='font-medium'>{names[provider]}</span>
                  {connected && (
                    <span className='text-muted-foreground text-sm'>
                      {t('Connected')}
                    </span>
                  )}
                  {connected && installation ? (
                    <Button
                      variant='outline'
                      disabled={action.isPending}
                      onClick={() =>
                        mutate(async () => {
                          await marketAPI.revokeProvider(installation, owner)
                        })
                      }
                    >
                      {t('Disconnect')}
                    </Button>
                  ) : (
                    <Button
                      variant='outline'
                      disabled={action.isPending}
                      onClick={() =>
                        mutate(async () => {
                          const start = await marketAPI.startProviderOAuth(
                            provider,
                            owner
                          )
                          const url = await prepareRemoteAuthorization(
                            owner,
                            start.installation,
                            start
                          )
                          const current = useAuthStore.getState().auth
                          if (
                            current.user?.id === userID &&
                            current.session?.sid === sessionId
                          )
                            window.location.assign(url)
                        })
                      }
                    >
                      {t('Connect')}
                    </Button>
                  )}
                </div>
                {connected && installation && (
                  <details className='text-sm'>
                    <summary>
                      {t('Tools')} ({tools.length})
                    </summary>
                    <Button
                      variant='outline'
                      disabled={action.isPending}
                      onClick={() =>
                        mutate(async () => {
                          await marketAPI.discoverProvider(installation, owner)
                        })
                      }
                    >
                      {t('Discover')}
                    </Button>
                    <ul className='my-2 space-y-1'>
                      {tools.map((name) => (
                        <li
                          key={name}
                          className='flex flex-wrap items-center justify-between gap-2'
                        >
                          <span>{name}</span>
                          <Button
                            disabled={!fixed_price_tools.includes(name)}
                            variant='ghost'
                            onClick={() => {
                              setSelected({ provider, name })
                              price.setInput('')
                              setSaved(false)
                            }}
                          >
                            {t('Save draft')}
                          </Button>
                        </li>
                      ))}
                    </ul>
                    {selected?.provider === provider && (
                      <form
                        className='space-y-2 py-2'
                        onSubmit={(event) => {
                          event.preventDefault()
                          if (price.quota === undefined) return
                          const quota = price.quota
                          mutate(async () => {
                            await marketAPI.importProviderTool(
                              provider,
                              selected.name,
                              quota,
                              owner
                            )
                            setSelected(undefined)
                            setSaved(true)
                          })
                        }}
                      >
                        <label className='space-y-1'>
                          <span>
                            {selected.name} · {t('Price')} ({label})
                          </span>
                          <Input
                            required
                            inputMode='decimal'
                            value={price.input}
                            onChange={(e) => price.setInput(e.target.value)}
                          />
                        </label>
                        <Button
                          type='submit'
                          disabled={
                            action.isPending || price.quota === undefined
                          }
                        >
                          {t('Save draft')}
                        </Button>
                      </form>
                    )}
                    {saved && (
                      <a className='underline' href='/tool-market?tab=mine'>
                        {t('My publications')}
                      </a>
                    )}
                  </details>
                )}
              </div>
            )
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}
