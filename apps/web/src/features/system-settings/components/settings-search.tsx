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
import { Link, useNavigate, useLocation } from '@tanstack/react-router'
import { ArrowUpRight, ChevronDown, ChevronRight, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { ROLE } from '@/lib/roles'
import {
  CORE_SETTINGS_HOME,
  getSettingsHomeForPath,
  getSettingsScope,
  getSettingsTitle,
} from '@/lib/system-settings-access'
import { useAuthStore } from '@/stores/auth-store'

import { useSystemOptions } from '../hooks/use-system-options'
import {
  buildSettingsSearchIndex,
  countSettingsEntries,
  searchSettings,
  settingsDestination,
  type SettingsSearchEntry,
} from '../utils/settings-search-index'

function useSettingsNavigation() {
  const { t } = useTranslation()
  const pathname = useLocation({ select: (location) => location.pathname })
  const search = useLocation({ select: (location) => location.searchStr })
  const role = useAuthStore((state) => state.auth.user?.role ?? ROLE.GUEST)
  const scope = getSettingsScope(pathname) ?? 'core'
  const options = useSystemOptions(role === ROLE.SUPER_ADMIN, false, 'session')
  const phase = useCapabilityFreshness(options)
  const available =
    role === ROLE.SUPER_ADMIN && phase === 'fresh' ? options.data : undefined
  const index = useMemo(
    () => buildSettingsSearchIndex(t, role, 'core', available),
    [t, role, available]
  )
  const current = index.find((group) =>
    group.entries.some((entry) => {
      const destination = settingsDestination(entry.url)
      return (
        destination.to === pathname &&
        (!destination.search?.section ||
          destination.search.section ===
            new URLSearchParams(search).get('section'))
      )
    })
  )
  return { index, current, pathname, search, scope }
}

export function SettingsBreadcrumb() {
  const { t } = useTranslation()
  const { current, pathname, search, scope } = useSettingsNavigation()
  const home = getSettingsHomeForPath(pathname) ?? CORE_SETTINGS_HOME
  return (
    <nav
      aria-label={t(getSettingsTitle(scope))}
      className='settings-breadcrumb text-muted-foreground flex flex-wrap items-center gap-2 text-xs'
    >
      <Link
        to={home}
        className='hover:text-foreground inline-flex min-h-9 items-center rounded-sm hover:underline'
      >
        {t(getSettingsTitle(scope))}
      </Link>
      {current && (
        <>
          <ChevronRight className='size-3' aria-hidden='true' />
          <SettingsPathMenu
            label={current.section}
            entries={current.entries}
            currentUrl={`${pathname}${search}`}
          />
        </>
      )}
    </nav>
  )
}

function SettingsPathMenu({
  label,
  entries,
  currentUrl,
  currentKind = 'page',
}: {
  label: string
  entries: SettingsSearchEntry[]
  currentUrl?: string
  currentKind?: 'page' | 'location'
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger className='hover:bg-muted hover:text-foreground focus-visible:ring-ring inline-flex min-h-8 min-w-0 items-center gap-1 rounded-md px-1.5 text-start outline-none focus-visible:ring-2'>
        {label}
        <ChevronDown className='size-3 shrink-0' aria-hidden='true' />
      </DropdownMenuTrigger>
      <DropdownMenuContent className='w-max max-w-[calc(100vw-2rem)] min-w-44 rounded-xl'>
        {entries.map((entry) => (
          <DropdownMenuItem
            key={entry.url}
            render={<Link {...settingsDestination(entry.url)} />}
            aria-current={currentUrl === entry.url ? currentKind : undefined}
            className='aria-[current]:bg-muted'
          >
            {entry.section}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function SettingsSearch() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { index, pathname, search } = useSettingsNavigation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const hasQuery = query.trim().length > 0
  const results = useMemo(
    () => (hasQuery ? searchSettings(index, query) : []),
    [hasQuery, index, query]
  )
  const activeUrl = `${pathname}${search}`

  return (
    <>
      <Button
        type='button'
        variant='outline'
        onClick={() => setOpen(true)}
        className='settings-search-trigger gap-2'
      >
        <Search className='size-4' aria-hidden='true' />
        <span>{t('Search settings')}</span>
      </Button>
      <CommandDialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          if (!next) setQuery('')
        }}
        className='top-[12dvh] sm:top-[15dvh]'
        title={t('Search settings')}
        description={t('Go to a settings section')}
      >
        {/*
          Filtering is owned by the index (titles, admin section, alternate
          terms and URL slug) so `shouldFilter` stays off and the ranking the
          user sees is the one `searchSettings` computed.
        */}
        <Command shouldFilter={false}>
          <CommandInput
            value={query}
            onValueChange={setQuery}
            placeholder={t('Search settings')}
          />
          <CommandList className='max-h-[min(65dvh,28rem)]'>
            {hasQuery && <CommandEmpty>{t('No settings found')}</CommandEmpty>}
            {results.map((group) => (
              <CommandGroup key={group.section} heading={group.group}>
                {group.entries.map((entry) => (
                  <CommandItem
                    key={entry.url}
                    value={`${group.group} ${entry.section} ${entry.url}`}
                    onSelect={() => {
                      setOpen(false)
                      setQuery('')
                      void navigate(settingsDestination(entry.url))
                    }}
                    className='gap-3 py-3'
                    aria-current={activeUrl === entry.url ? 'page' : undefined}
                  >
                    <span className='flex-1'>{entry.section}</span>
                    <span className='text-muted-foreground text-xs'>
                      {group.group}
                    </span>
                    <ArrowUpRight
                      className='text-muted-foreground size-3.5'
                      aria-hidden='true'
                    />
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
          {hasQuery && (
            <div className='text-muted-foreground border-border/70 border-t px-3 py-2 text-xs'>
              {t('{{count}} sections', {
                count: countSettingsEntries(results),
              })}
            </div>
          )}
        </Command>
      </CommandDialog>
    </>
  )
}
