/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link } from '@tanstack/react-router'
import { ChevronDown, Copy, ListFilter } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { copyToClipboard } from '@/lib/copy-to-clipboard'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

type EntityKind = 'username' | 'channel' | 'token' | 'model'

export function EntityQuickActions({
  kind,
  value,
  label = value,
  hidden = false,
  onDetails,
  leading,
}: {
  kind: EntityKind
  value: string
  label?: string
  hidden?: boolean
  onDetails?: () => void
  leading?: ReactNode
}) {
  const { t } = useTranslation()
  const isAdmin = useAuthStore(
    (state) => (state.auth.user?.role ?? 0) >= ROLE.ADMIN
  )
  if (!value || hidden) return <span className='truncate'>{label}</span>
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <button
            type='button'
            onClick={(event) => event.stopPropagation()}
            className='text-primary hover:bg-primary/5 focus-visible:ring-ring inline-flex min-h-11 max-w-full items-center gap-1 rounded px-1 text-left focus-visible:ring-2 focus-visible:outline-none sm:min-h-8'
          />
        }
      >
        {leading}
        <span className='truncate'>{label}</span>
        <ChevronDown
          className='size-3 shrink-0 opacity-60'
          aria-hidden='true'
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent onClick={(event) => event.stopPropagation()}>
        {onDetails && (
          <DropdownMenuItem onSelect={onDetails}>
            {t('Details')}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          render={
            <Link
              to='/usage-logs/$section'
              params={{ section: 'common' }}
              search={{ [kind]: value }}
            />
          }
        >
          <ListFilter className='size-4' />
          {t('View logs')}
        </DropdownMenuItem>
        {isAdmin && kind === 'username' && (
          <DropdownMenuItem
            render={<Link to='/users' search={{ keyword: value }} />}
          >
            {t('Users')}
          </DropdownMenuItem>
        )}
        {isAdmin && kind === 'channel' && (
          <DropdownMenuItem
            render={<Link to='/channels' search={{ filter: value }} />}
          >
            {t('Channels')}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          onSelect={async () => {
            const copied = await copyToClipboard(value)
            if (copied) toast.success(t('Copied'))
            else toast.error(t('Failed to copy'))
          }}
        >
          <Copy className='size-4' />
          {t('Copy')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
