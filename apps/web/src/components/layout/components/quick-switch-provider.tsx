/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useLocation, useNavigate } from '@tanstack/react-router'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'

import { useModelPlaza } from '@/context/model-plaza-provider'
import { useRootSidebarNavigation } from '@/hooks/use-sidebar-view'

import { findConsoleNavItem } from '../lib/console-navigation'
import { requestPaletteOpen } from '../lib/quick-switch-events'
import { CONSOLE_SHORTCUTS, type ShortcutDefinition } from '../lib/shortcuts'

const RECENT_ROUTES_STORAGE_KEY = 'lmm.recent-routes'
const RECENT_ROUTES_LIMIT = 5

type QuickSwitchContextValue = {
  /** Recently visited console paths, newest first. */
  recentRoutes: string[]
  /** Available shortcuts after role, module and runtime capability filtering. */
  shortcuts: readonly ShortcutDefinition[]
  /** Shared dispatch for key sequences and the shortcut sheet. */
  runNavigationShortcut: (to: string) => boolean
  /** Ctrl/⌘ K — the same palette the search provider owns. */
  openPalette: () => void
  /** `?` — the keyboard shortcut cheat sheet. */
  openShortcuts: () => void
  shortcutSheetOpen: boolean
  setShortcutSheetOpen: (open: boolean) => void
}

const QuickSwitchContext = createContext<QuickSwitchContextValue | null>(null)

function readStoredRoutes(): string[] {
  if (typeof window === 'undefined') return []
  try {
    const raw = window.localStorage.getItem(RECENT_ROUTES_STORAGE_KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter((entry): entry is string => typeof entry === 'string')
  } catch {
    return []
  }
}

function recordRoute(pathname: string, routes: string[]): string[] {
  return [pathname, ...routes.filter((entry) => entry !== pathname)].slice(
    0,
    RECENT_ROUTES_LIMIT
  )
}

function isTypingTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false
  return Boolean(
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"])'
    )
  )
}

/**
 * Console quick-switch state: recent visits, the shared palette opener and the
 * `?` shortcut sheet.
 *
 * Mounted by the signed-in shell only; public pages keep their own behaviour.
 */
export function QuickSwitchProvider({ children }: { children: ReactNode }) {
  const [shortcutSheetOpen, setShortcutSheetOpen] = useState(false)
  const pathname = useLocation({ select: (location) => location.pathname })
  const [history, setHistory] = useState(() => ({
    pathname,
    routes: recordRoute(pathname, readStoredRoutes()),
  }))
  if (history.pathname !== pathname) {
    setHistory({ pathname, routes: recordRoute(pathname, history.routes) })
  }
  const recentRoutes = history.routes
  const navigate = useNavigate()
  const { openPanel } = useModelPlaza()
  const navGroups = useRootSidebarNavigation()
  const shortcuts = useMemo(
    () =>
      CONSOLE_SHORTCUTS.filter((shortcut) => {
        if (!shortcut.to) return true
        const item = findConsoleNavItem(navGroups, shortcut.to)
        return item && !item.disabled
      }),
    [navGroups]
  )
  const runNavigationShortcut = useCallback(
    (to: string) => {
      const item = findConsoleNavItem(navGroups, to)
      if (!item || item.disabled) return false
      if (item.interaction === 'model-panel') openPanel()
      else void navigate({ to })
      return true
    },
    [navGroups, navigate, openPanel]
  )
  const pendingPrefixRef = useRef<number | null>(null)

  // Persist the bounded history after the current route has committed.
  useEffect(() => {
    try {
      window.localStorage.setItem(
        RECENT_ROUTES_STORAGE_KEY,
        JSON.stringify(recentRoutes)
      )
    } catch {
      // Storage can be unavailable (private mode); recents stay in memory.
    }
  }, [recentRoutes])

  const openPalette = useCallback(() => requestPaletteOpen(), [])

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || event.isComposing) return
      if (event.key !== '?' || event.metaKey || event.ctrlKey || event.altKey) {
        return
      }
      if (isTypingTarget(event.target)) return
      if (
        document.querySelector(
          '[role="dialog"], [role="alertdialog"], [role="menu"]'
        )
      ) {
        return
      }
      event.preventDefault()
      setShortcutSheetOpen((open) => !open)
    }
    document.addEventListener('keydown', down)
    return () => document.removeEventListener('keydown', down)
  }, [])

  // `G` then a letter jumps straight to a primary console page.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey
      ) {
        return
      }
      if (
        isTypingTarget(event.target) ||
        document.querySelector(
          '[role="dialog"], [role="alertdialog"], [role="menu"]'
        )
      ) {
        pendingPrefixRef.current = null
        return
      }
      const key = event.key.toLowerCase()
      const prefixTime = pendingPrefixRef.current
      if (prefixTime !== null && Date.now() - prefixTime < 1000) {
        pendingPrefixRef.current = null
        const destination = shortcuts.find(
          (shortcut) =>
            shortcut.group === 'navigation' &&
            shortcut.keys[1]?.toLowerCase() === key
        )
        if (destination?.to && runNavigationShortcut(destination.to)) {
          event.preventDefault()
        }
        return
      }
      if (key === 'g') {
        pendingPrefixRef.current = Date.now()
      } else {
        pendingPrefixRef.current = null
      }
    }
    const reset = () => {
      pendingPrefixRef.current = null
    }
    document.addEventListener('keydown', down)
    document.addEventListener('pointerdown', reset)
    return () => {
      document.removeEventListener('keydown', down)
      document.removeEventListener('pointerdown', reset)
    }
  }, [runNavigationShortcut, shortcuts])

  const value = useMemo<QuickSwitchContextValue>(
    () => ({
      recentRoutes,
      shortcuts,
      runNavigationShortcut,
      openPalette,
      openShortcuts: () => setShortcutSheetOpen(true),
      shortcutSheetOpen,
      setShortcutSheetOpen,
    }),
    [
      openPalette,
      recentRoutes,
      shortcuts,
      runNavigationShortcut,
      shortcutSheetOpen,
    ]
  )

  return (
    <QuickSwitchContext.Provider value={value}>
      {children}
    </QuickSwitchContext.Provider>
  )
}

// eslint-disable-next-line react-refresh/only-export-components
export function useQuickSwitch() {
  const context = useContext(QuickSwitchContext)
  if (!context) {
    throw new Error('useQuickSwitch has to be used within QuickSwitchProvider')
  }
  return context
}

// eslint-disable-next-line react-refresh/only-export-components
export function useQuickSwitchOptional() {
  return useContext(QuickSwitchContext)
}

export type { QuickSwitchContextValue }
