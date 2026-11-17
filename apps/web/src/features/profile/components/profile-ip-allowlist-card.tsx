/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Network } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { TitledCard } from '@/components/ui/titled-card'
import { useAuthStore } from '@/stores/auth-store'

import {
  getIPAllowlist,
  IPAllowlistFailure,
  isCurrentIPAllowlistScope,
  parseIPAllowlistDraft,
  saveIPAllowlist,
  type IPAllowlistScope,
} from '../ip-allowlist-api'

export function ProfileIPAllowlistCard() {
  const auth = useAuthStore((state) => state.auth)
  if (!auth.user || !auth.session) return null
  const scope = { userId: auth.user.id, sessionId: auth.session.sid }
  return (
    <IPAllowlistWorkspace
      key={`${scope.userId}:${scope.sessionId}`}
      scope={scope}
    />
  )
}

function IPAllowlistWorkspace({ scope }: { scope: IPAllowlistScope }) {
  const { t } = useTranslation()
  const inputId = useId()
  const descriptionId = useId()
  const errorId = useId()
  const queryClient = useQueryClient()
  const queryKey = ['profile', 'ip-allowlist', scope.userId, scope.sessionId]
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState('')
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => getIPAllowlist(scope, signal),
    refetchOnWindowFocus: false,
  })
  const mutation = useMutation({
    mutationFn: (entries: string[]) => saveIPAllowlist(scope, entries),
    retry: false,
  })
  const saved = query.data?.allow_ips.join('\n') ?? ''
  const value = draft ?? saved
  const entries = parseIPAllowlistDraft(value)
  const currentIP = query.data?.client_ip
  const dirty = entries.join('\n') !== saved

  async function save(event: FormEvent) {
    event.preventDefault()
    setError('')
    if (entries.length > 128) {
      setError(t('Use no more than 128 IP addresses or networks.'))
      return
    }
    try {
      const policy = await mutation.mutateAsync(entries)
      if (!isCurrentIPAllowlistScope(scope)) return
      queryClient.setQueryData(queryKey, policy)
      setDraft(null)
      toast.success(t('IP allowlist saved'))
    } catch (cause) {
      if (!isCurrentIPAllowlistScope(scope)) return
      const message =
        cause instanceof IPAllowlistFailure &&
        cause.code === 'USER_IP_ALLOWLIST_LOCKOUT'
          ? t(
              'Include your current IP before saving to keep access to your account.'
            )
          : cause instanceof IPAllowlistFailure &&
              cause.code === 'USER_IP_ALLOWLIST_INVALID'
            ? t('Enter valid IPv4, IPv6 or CIDR networks, one per line.')
            : t(
                'Unable to confirm the IP allowlist update. Try again or reload.'
              )
      setError(message)
      if (
        cause instanceof IPAllowlistFailure &&
        cause.code === 'USER_IP_ALLOWLIST_LOCKOUT'
      ) {
        void query.refetch()
      }
    }
  }

  return (
    <TitledCard
      title={t('IP allowlist')}
      description={t(
        'Restrict new logins, active sessions and API requests to your trusted IP addresses.'
      )}
      icon={<Network className='h-4 w-4' />}
      disableHoverEffect
    >
      {query.isPending ? (
        <Skeleton className='h-44 w-full' />
      ) : query.isError ? (
        <div role='alert' className='space-y-3'>
          <p className='text-destructive text-sm'>
            {t('Unable to load IP allowlist')}
          </p>
          <Button variant='outline' onClick={() => void query.refetch()}>
            {t('Retry')}
          </Button>
        </div>
      ) : (
        <form onSubmit={(event) => void save(event)} className='space-y-5'>
          <div className='flex flex-wrap items-center justify-between gap-3'>
            <div className='min-w-0 space-y-1'>
              <p className='text-muted-foreground text-xs'>
                {t('Current IP detected by the server')}
              </p>
              <p className='font-mono text-sm break-all'>
                {currentIP ?? t('Unavailable')}
              </p>
            </div>
            <Button
              type='button'
              variant='outline'
              disabled={!currentIP || mutation.isPending}
              onClick={() => {
                if (currentIP) {
                  setDraft([...new Set([...entries, currentIP])].join('\n'))
                  setError('')
                }
              }}
            >
              {t('Add current IP')}
            </Button>
          </div>
          <div className='space-y-2'>
            <Label htmlFor={inputId}>
              {t('Allowed IP addresses and networks')}
            </Label>
            <Textarea
              id={inputId}
              value={value}
              onChange={(event) => {
                setDraft(event.target.value)
                setError('')
              }}
              disabled={mutation.isPending}
              aria-describedby={`${descriptionId}${error ? ` ${errorId}` : ''}`}
              aria-invalid={Boolean(error)}
              className='min-h-36 font-mono text-sm leading-6'
              rows={5}
              autoCapitalize='none'
              autoCorrect='off'
              spellCheck={false}
              placeholder={'203.0.113.8\n198.51.100.0/24\n2001:db8::/32'}
            />
            <p
              id={descriptionId}
              className='text-muted-foreground text-sm leading-6'
            >
              {t(
                'Enter IPv4, IPv6 or CIDR networks, one per line. Leave empty to allow any IP.'
              )}
            </p>
            <p className='text-muted-foreground text-sm leading-6'>
              {t(
                'Your allowlist is checked together with platform region rules and each API key’s IP restrictions.'
              )}
            </p>
            {error && (
              <p id={errorId} role='alert' className='text-destructive text-sm'>
                {error}
              </p>
            )}
          </div>
          <div className='flex flex-wrap items-center gap-3'>
            <Button type='submit' disabled={!dirty || mutation.isPending}>
              {mutation.isPending ? t('Saving...') : t('Save changes')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              disabled={!dirty || mutation.isPending}
              onClick={() => {
                setDraft(null)
                setError('')
              }}
            >
              {t('Reset')}
            </Button>
          </div>
        </form>
      )}
    </TitledCard>
  )
}
