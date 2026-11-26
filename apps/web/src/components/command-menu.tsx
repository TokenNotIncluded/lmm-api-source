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
import { useNavigate } from '@tanstack/react-router'
import {
  ArrowRight,
  ChevronRight,
  Clock,
  Keyboard,
  Laptop,
  Moon,
  Sun,
} from 'lucide-react'
import React from 'react'
import { useTranslation } from 'react-i18next'

import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '@/components/ui/command'
import { useModelPlaza } from '@/context/model-plaza-provider'
import { useSearch } from '@/context/search-provider'
import { useTheme } from '@/context/theme-provider'
import { useRootSidebarNavigation } from '@/hooks/use-sidebar-view'
import appI18n from '@/i18n/config'
import {
  INTERFACE_LANGUAGE_OPTIONS,
  normalizeInterfaceLanguage,
} from '@/i18n/languages'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { useQuickSwitch } from './layout/components/quick-switch-provider'
import { findConsoleNavItem } from './layout/lib/console-navigation'
import { onPaletteOpenRequest } from './layout/lib/quick-switch-events'

export function CommandMenu() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { setTheme } = useTheme()
  const { open, setOpen } = useSearch()
  const { openPanel } = useModelPlaza()
  const quickSwitch = useQuickSwitch()
  const signedInUser = useAuthStore((state) => state.auth.user)
  // Search must respect the same role and module filters as the visible sidebar.
  const navGroups = useRootSidebarNavigation()
  const currentLanguage = normalizeInterfaceLanguage(appI18n.language)

  const runCommand = React.useCallback(
    (command: () => unknown) => {
      setOpen(false)
      command()
    },
    [setOpen]
  )

  // Other shell surfaces (sidebar, shortcut sheet) ask for the palette by event
  // so they never need to import the search context.
  React.useEffect(() => onPaletteOpenRequest(() => setOpen(true)), [setOpen])

  const recentEntries = quickSwitch.recentRoutes
    .flatMap((pathname) => {
      const item = findConsoleNavItem(navGroups, pathname)
      return item && !item.disabled
        ? [{ pathname, label: item.title, interaction: item.interaction }]
        : []
    })
    .slice(0, 5)

  // Mirrors the header language switcher: switch immediately, persist best-effort.
  const changeLanguage = async (code: string) => {
    await appI18n.changeLanguage(code)
    if (signedInUser) {
      try {
        await api.put('/api/user/self', { language: code })
      } catch {
        // Best-effort persistence; don't block the UI on failure
      }
    }
  }

  return (
    <CommandDialog
      modal
      open={open}
      onOpenChange={setOpen}
      title={t('Search')}
      description={t('Type a command or search...')}
    >
      <Command className='[&_[cmdk-item]]:min-h-11'>
        <CommandInput placeholder={t('Type a command or search...')} />
        <CommandList>
          <CommandEmpty>{t('No results found.')}</CommandEmpty>
          <CommandGroup heading={t('Quick actions')}>
            <CommandItem
              value={`${t('Keyboard shortcuts')} shortcuts`}
              onSelect={() => runCommand(() => quickSwitch.openShortcuts())}
            >
              <Keyboard />
              <span>{t('Keyboard shortcuts')}</span>
            </CommandItem>
          </CommandGroup>
          {recentEntries.length > 0 && (
            <>
              <CommandSeparator />
              <CommandGroup heading={t('Recently visited')}>
                {recentEntries.map((entry) => (
                  <CommandItem
                    key={`recent-${entry.pathname}`}
                    value={`recent ${entry.label} ${entry.pathname}`}
                    onSelect={() =>
                      runCommand(() =>
                        entry.interaction === 'model-panel'
                          ? openPanel()
                          : navigate({ to: entry.pathname })
                      )
                    }
                  >
                    <Clock />
                    <span>{entry.label}</span>
                  </CommandItem>
                ))}
              </CommandGroup>
            </>
          )}
          {navGroups.map((group) => (
            <CommandGroup key={group.id || group.title} heading={group.title}>
              {group.items.map((navItem, i) => {
                if (navItem.url) {
                  return (
                    <CommandItem
                      key={`${navItem.url}-${i}`}
                      value={`${navItem.title} ${navItem.url}`}
                      keywords={
                        navItem.url === '/wallet' ? [t('Top up')] : undefined
                      }
                      disabled={navItem.disabled}
                      onSelect={() => {
                        runCommand(() =>
                          navItem.interaction === 'model-panel'
                            ? openPanel()
                            : navigate({ to: navItem.url })
                        )
                      }}
                    >
                      <div className='flex size-4 items-center justify-center'>
                        <ArrowRight className='text-muted-foreground/80 size-2' />
                      </div>
                      <span className='min-w-0 break-words'>
                        {navItem.title}
                      </span>
                    </CommandItem>
                  )
                }

                return navItem.items?.map((subItem, i) => (
                  <CommandItem
                    key={`${navItem.title}-${subItem.url}-${i}`}
                    value={`${navItem.title} ${subItem.title} ${subItem.url}`}
                    disabled={navItem.disabled || subItem.disabled}
                    onSelect={() => {
                      runCommand(() =>
                        subItem.interaction === 'model-panel'
                          ? openPanel()
                          : navigate({ to: subItem.url })
                      )
                    }}
                  >
                    <div className='flex size-4 items-center justify-center'>
                      <ArrowRight className='text-muted-foreground/80 size-2' />
                    </div>
                    <span className='min-w-0 break-words'>{navItem.title}</span>
                    <ChevronRight aria-hidden='true' />
                    <span className='min-w-0 break-words'>{subItem.title}</span>
                  </CommandItem>
                ))
              })}
            </CommandGroup>
          ))}
          <CommandSeparator />
          <CommandGroup heading={t('Language')}>
            {INTERFACE_LANGUAGE_OPTIONS.map((language) => (
              <CommandItem
                key={`language-${language.code}`}
                value={`language ${language.label}`}
                onSelect={() =>
                  runCommand(() => {
                    void changeLanguage(language.code)
                  })
                }
                aria-current={
                  currentLanguage === language.code ? 'true' : undefined
                }
              >
                <span className='w-4 text-center text-xs'>
                  {currentLanguage === language.code ? '✓' : ''}
                </span>
                <span>{language.label}</span>
              </CommandItem>
            ))}
          </CommandGroup>
          <CommandSeparator />
          <CommandGroup heading={t('Theme')}>
            <CommandItem onSelect={() => runCommand(() => setTheme('light'))}>
              <Sun /> <span>{t('Light')}</span>
            </CommandItem>
            <CommandItem onSelect={() => runCommand(() => setTheme('dark'))}>
              <Moon className='scale-90' />
              <span>{t('Dark')}</span>
            </CommandItem>
            <CommandItem onSelect={() => runCommand(() => setTheme('system'))}>
              <Laptop />
              <span>{t('System')}</span>
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
