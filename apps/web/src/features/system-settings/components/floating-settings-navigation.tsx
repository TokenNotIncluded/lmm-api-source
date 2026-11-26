/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link, useLocation, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, ChevronDown, Menu, Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { getSystemSettingsNavGroups } from '@/components/layout/config/system-settings.config'
import { Input } from '@/components/ui/input'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { useSidebar } from '@/components/ui/sidebar'
import { ROLE } from '@/lib/roles'
import {
  CORE_SETTINGS_HOME,
  EXTENSION_SETTINGS_HOME,
  getSettingsScope,
  getSettingsTitle,
} from '@/lib/system-settings-access'
import { useAuthStore } from '@/stores/auth-store'

import { mountSettingsScrubber } from '../utils/settings-scrubber'
import {
  buildSettingsSearchIndex,
  searchSettings,
} from '../utils/settings-search-index'

import './floating-settings-navigation.css'

/** Shares the settings route registry, sidebar trigger and route-change guards. */
export function FloatingSettingsNavigation() {
  const { t } = useTranslation()
  const { open, setOpen, openMobile, setOpenMobile, isMobile } = useSidebar()
  const { pathname, searchStr } = useLocation()
  const navigate = useNavigate()
  const [waitingForSidebarReset, setWaitingForSidebarReset] = useState(
    open || openMobile
  )
  if (waitingForSidebarReset && !open && !openMobile) {
    setWaitingForSidebarReset(false)
  }
  const [query, setQuery] = useState('')
  const [preview, setPreview] = useState<number | null>(null)
  const handle = useRef<HTMLButtonElement>(null)
  const sidebar = useRef({ open, setOpen, setOpenMobile })
  useEffect(() => {
    sidebar.current = { open, setOpen, setOpenMobile }
  }, [open, setOpen, setOpenMobile])
  const role = useAuthStore((state) => state.auth.user?.role ?? ROLE.GUEST)
  const scope = getSettingsScope(pathname)
  const title = t(getSettingsTitle(scope))
  const otherScope = scope === 'core' ? 'extensions' : 'core'
  const index = useMemo(
    () =>
      buildSettingsSearchIndex(
        t,
        getSystemSettingsNavGroups(t, role, scope),
        role
      ),
    [t, role, scope]
  )
  const entries = useMemo(
    () => index.flatMap((group) => group.entries),
    [index]
  )
  const currentIndex = Math.max(
    0,
    entries.findIndex((entry) => {
      const [path, query] = entry.url.split('?')
      const actual = new URLSearchParams(searchStr)
      return (
        path === pathname &&
        [...new URLSearchParams(query)].every(
          ([key, value]) => actual.get(key) === value
        )
      )
    })
  )
  const current = entries[currentIndex]
  const shown = !waitingForSidebarReset && (isMobile ? openMobile : open)
  const setShown = useCallback(
    (value: boolean) => {
      if (isMobile) setOpenMobile(value)
      else setOpen(value)
    },
    [isMobile, setOpen, setOpenMobile]
  )
  const scopeLink = role === ROLE.SUPER_ADMIN && (
    <Link
      to={scope === 'core' ? EXTENSION_SETTINGS_HOME : CORE_SETTINGS_HOME}
      className='settings-desktop-back'
      onClick={() => setShown(false)}
    >
      {t(getSettingsTitle(otherScope))}
    </Link>
  )
  const select = useCallback(
    (position: number) => {
      const entry = entries[position]
      if (!entry) return
      setShown(false)
      const [to, query] = entry.url.split('?')
      void navigate({
        to,
        search: query
          ? Object.fromEntries(new URLSearchParams(query))
          : undefined,
      })
    },
    [entries, navigate, setShown]
  )

  useEffect(() => {
    const button = handle.current
    if (!button) return
    return mountSettingsScrubber(
      button,
      currentIndex,
      entries.length,
      setPreview,
      select,
      () => setShown(true)
    )
  }, [currentIndex, entries.length, select, setShown])

  // Client-side entry can inherit an open console sidebar. Keep the modal
  // closed from its first render, not just after this effect resets that state.
  useEffect(() => {
    const previous = sidebar.current.open
    sidebar.current.setOpen(false)
    sidebar.current.setOpenMobile(false)
    return () => {
      sidebar.current.setOpen(previous)
      sidebar.current.setOpenMobile(false)
    }
  }, [])

  const results = searchSettings(index, query)
  const directory = (
    <nav
      className='settings-directory-list'
      aria-label={t('Go to a settings section')}
    >
      {results.length === 0 ? (
        <p className='text-muted-foreground px-3 py-6'>
          {t('No settings found')}
        </p>
      ) : null}
      {results.map((group) => (
        <details
          key={`${group.group}:${query}`}
          open={
            query.trim()
              ? true
              : group.entries.some((entry) => entry.url === current?.url)
          }
        >
          <summary>
            <span>{group.group}</span>
            <ChevronDown aria-hidden='true' className='size-4' />
          </summary>
          {group.entries.map((entry) => (
            <button
              key={entry.url}
              type='button'
              aria-current={entry.url === current?.url ? 'page' : undefined}
              onClick={() =>
                select(entries.findIndex((item) => item.url === entry.url))
              }
            >
              {entry.section}
            </button>
          ))}
        </details>
      ))}
    </nav>
  )
  return (
    <>
      <aside className='settings-desktop-navigation' aria-label={title}>
        <Link
          to='/dashboard/$section'
          params={{ section: 'overview' }}
          className='settings-desktop-back'
        >
          <ArrowLeft aria-hidden='true' className='size-4' />
          <span>{t('Back to Dashboard')}</span>
        </Link>
        <h2 className='settings-directory-heading'>{title}</h2>
        <div className='settings-directory-search'>
          <Search aria-hidden='true' className='size-4' />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('Search settings')}
            aria-label={t('Search settings')}
          />
        </div>
        {directory}
        {scopeLink}
      </aside>
      <aside className='settings-float-rail' aria-label={title}>
        <button
          ref={handle}
          type='button'
          className='settings-scrub-handle'
          aria-label={t(
            'Open settings directory; hold and drag to switch sections'
          )}
          aria-haspopup='dialog'
          aria-expanded={shown}
          title={t(
            'Hold and drag up or down. Release to open the selected section.'
          )}
        >
          <Menu aria-hidden='true' className='size-4' />
          <span>{t('Directory')}</span>
        </button>
        <output
          className='settings-scrub-preview'
          hidden={preview === null}
          aria-live='polite'
        >
          {preview !== null ? (
            <>
              <small>{entries[preview]?.group}</small>
              <strong>{entries[preview]?.section}</strong>
              <span>{t('Release to open')}</span>
            </>
          ) : null}
        </output>
      </aside>
      <Sheet open={shown} onOpenChange={setShown}>
        <SheetContent
          side={isMobile ? 'bottom' : 'left'}
          className='settings-floating-directory'
          finalFocus={isMobile ? handle : undefined}
        >
          <SheetHeader className='px-6 pt-6 pb-0'>
            <SheetTitle>{title}</SheetTitle>
            <SheetDescription>
              {current?.group} · {current?.section}
            </SheetDescription>
          </SheetHeader>
          <div className='settings-directory-search settings-directory-search-modal'>
            <Search aria-hidden='true' className='size-4' />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t('Search settings')}
              aria-label={t('Search settings')}
            />
          </div>
          {directory}
          <div className='settings-directory-footer'>
            {scopeLink}
            <Link
              to='/dashboard/$section'
              params={{ section: 'overview' }}
              className='settings-desktop-back'
              onClick={() => setShown(false)}
            >
              <ArrowLeft aria-hidden='true' className='size-4' />
              <span>{t('Back to Dashboard')}</span>
            </Link>
          </div>
        </SheetContent>
      </Sheet>
    </>
  )
}
