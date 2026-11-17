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
along with this program. If not, you have received a copy of the
License, or (at your option) any later version.
*/
import { AiChat02Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { lazy, Suspense, useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useStatus } from '@/hooks/use-status'
import { isConsoleActivated } from '@/lib/console-activation'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import {
  consumeQueuedAssistantRequest,
  peekQueuedAssistantRequest,
  subscribeToAssistantOpen,
  type AssistantPresetId,
} from './assistant-events'
import { setAssistantRailOpen, useAssistantRailOpen } from './assistant-rail'
import { useAssistantOverlay } from './assistant-responsive'

const loadAssistantPanel = () => import('./assistant-panel')
const AssistantPanel = lazy(() =>
  loadAssistantPanel().then((module) => ({
    default: module.AssistantPanel,
  }))
)

/** Desktop rail width. Kept in one place so the animated wrapper and the
 * panel itself stay perfectly in sync while the width transition runs. */
// `w-96 max-w-[26vw]` avoids commas inside arbitrary values, which this
// Tailwind setup does not generate rules for.
const RAIL_WIDTH = 'w-96 max-w-[26vw]'

type LauncherRequestState = {
  ownerKey: string
  initialPreset?: AssistantPresetId
  initialMessage?: string
  initialMessageRevision: number
  autoSendRequestId?: string
}

export function AssistantLauncher(props: {
  page?: boolean
  hideMobileLauncher?: boolean
}) {
  const { t } = useTranslation()
  const { status } = useStatus()
  const { user, session } = useAuthStore((state) => state.auth)
  const hasUser = Boolean(user)
  const ownerKey = JSON.stringify([user?.id ?? null, session?.sid ?? null])
  const [mountedRequest] = useState(() => ({
    ownerKey,
    request: peekQueuedAssistantRequest(),
  }))
  const isOverlayViewport = useAssistantOverlay()
  const [mobileOpen, setMobileOpen] = useState(Boolean(mountedRequest.request))
  const railOpen = useAssistantRailOpen()
  const defaultPreset: AssistantPresetId | undefined = props.page
    ? isConsoleActivated(user)
      ? 'client-setup'
      : 'onboarding'
    : undefined
  const [requestState, setRequestState] = useState<LauncherRequestState>(() => {
    const queued = mountedRequest.request
    return {
      ownerKey,
      initialPreset: queued
        ? (queued.preset ??
          (queued.autoSend
            ? isConsoleActivated(user)
              ? 'service'
              : 'onboarding'
            : undefined))
        : defaultPreset,
      initialMessage: queued?.message,
      initialMessageRevision: queued?.message?.trim() ? 1 : 0,
      autoSendRequestId: queued?.autoSend ? queued.id : undefined,
    }
  })
  if (requestState.ownerKey !== ownerKey) {
    setRequestState({
      ownerKey,
      initialPreset: defaultPreset,
      initialMessage: undefined,
      initialMessageRevision: requestState.initialMessageRevision + 1,
      autoSendRequestId: undefined,
    })
    setMobileOpen(false)
  }
  const {
    initialPreset,
    initialMessage,
    initialMessageRevision,
    autoSendRequestId,
  } = requestState
  const desktopScope = JSON.stringify([
    ownerKey,
    railOpen && !isOverlayViewport,
  ])
  const [desktop, setDesktop] = useState({
    scope: desktopScope,
    fullscreen: false,
  })
  if (desktop.scope !== desktopScope) {
    setDesktop({ scope: desktopScope, fullscreen: false })
  }
  const desktopFullscreen = desktop.scope === desktopScope && desktop.fullscreen

  const showAssistant = useCallback(
    (request: {
      id: string
      preset?: AssistantPresetId
      message?: string
      autoSend: boolean
    }) => {
      const current = useAuthStore.getState().auth
      if (
        JSON.stringify([
          current.user?.id ?? null,
          current.session?.sid ?? null,
        ]) !== ownerKey
      ) {
        return
      }
      let preset = request.preset
      if (!preset && request.autoSend) {
        preset = isConsoleActivated(user) ? 'service' : 'onboarding'
      }
      setRequestState((current) => ({
        ownerKey,
        initialPreset: preset,
        initialMessage: request.message,
        initialMessageRevision:
          current.initialMessageRevision + (request.message?.trim() ? 1 : 0),
        autoSendRequestId:
          request.message?.trim() && request.autoSend ? request.id : undefined,
      }))
      if (!request.autoSend) consumeQueuedAssistantRequest(request.id)
      // Desktop: the in-flow rail opens (the panel itself keeps its
      // conversation state because it never unmounts). Mobile: overlay.
      setAssistantRailOpen(true)
      setMobileOpen(true)
    },
    [ownerKey, user]
  )

  // The overlay sheet and the desktop rail are exclusive by viewport; only
  // the active one mounts so a queued auto-send can never fire twice.

  const handleConversationReset = useCallback(() => {
    setRequestState((current) =>
      current.ownerKey === ownerKey
        ? {
            ownerKey,
            initialPreset: undefined,
            initialMessage: undefined,
            initialMessageRevision: current.initialMessageRevision + 1,
            autoSendRequestId: undefined,
          }
        : current
    )
  }, [ownerKey])

  useEffect(() => {
    const queued = mountedRequest.request
    const current = useAuthStore.getState().auth
    if (
      !queued ||
      JSON.stringify([
        current.user?.id ?? null,
        current.session?.sid ?? null,
      ]) !== mountedRequest.ownerKey
    ) {
      return
    }
    setAssistantRailOpen(true)
    if (!queued.autoSend) consumeQueuedAssistantRequest(queued.id)
  }, [mountedRequest])

  useEffect(() => subscribeToAssistantOpen(showAssistant), [showAssistant])

  useEffect(
    () => () => {
      const current = useAuthStore.getState().auth
      const currentKey = JSON.stringify([
        current.user?.id ?? null,
        current.session?.sid ?? null,
      ])
      if (hasUser && currentKey !== ownerKey && autoSendRequestId) {
        consumeQueuedAssistantRequest(autoSendRequestId)
      }
    },
    [autoSendRequestId, ownerKey, hasUser]
  )

  const handleAutoSendConsumed = useCallback(
    (requestId: string) => {
      consumeQueuedAssistantRequest(requestId)
      setRequestState((current) =>
        current.ownerKey === ownerKey && current.autoSendRequestId === requestId
          ? { ...current, autoSendRequestId: undefined }
          : current
      )
    },
    [ownerKey]
  )

  const showManualAssistant = useCallback(() => {
    showAssistant({ id: 'manual', autoSend: false })
  }, [showAssistant])

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.isComposing ||
        event.repeat ||
        (status?.assistant?.enabled === false && !user) ||
        event.altKey ||
        !event.shiftKey ||
        !(event.metaKey || event.ctrlKey) ||
        (event.code !== 'Space' && event.key.toLowerCase() !== 'a')
      ) {
        return
      }

      event.preventDefault()
      if (!props.page && (isOverlayViewport ? mobileOpen : railOpen)) {
        if (isOverlayViewport) setMobileOpen(false)
        else setAssistantRailOpen(false)
      } else {
        showManualAssistant()
      }
    }

    window.addEventListener('keydown', handleShortcut)
    return () => window.removeEventListener('keydown', handleShortcut)
  }, [
    showManualAssistant,
    props.page,
    isOverlayViewport,
    mobileOpen,
    railOpen,
    status?.assistant?.enabled,
    user,
  ])

  if (status?.assistant?.enabled === false && !user) return null

  if (props.page) {
    return (
      <Suspense
        fallback={<aside className='bg-background hidden' aria-hidden='true' />}
      >
        <AssistantPanel
          mode='page'
          open
          initialPreset={initialPreset}
          initialMessage={initialMessage}
          initialMessageRevision={initialMessageRevision}
          autoSendRequestId={autoSendRequestId}
          onAutoSendConsumed={handleAutoSendConsumed}
          onOpenChange={() => undefined}
          onConversationReset={handleConversationReset}
        />
      </Suspense>
    )
  }

  const visibleLabel = t('AI assistant')
  const accessibleLabel = t('Open AI assistant')

  return (
    <div className='contents'>
      {/* Mobile floating pill — opens the overlay sheet. */}
      {!props.hideMobileLauncher ? (
        <div
          className='pointer-events-none fixed inset-x-0 bottom-0 z-40 flex min-h-14 items-center justify-end px-3 py-1.5 pb-[max(0.375rem,env(safe-area-inset-bottom))] xl:hidden'
          data-testid='assistant-mobile-launcher'
        >
          <Button
            type='button'
            variant='secondary'
            className='border-primary/20 bg-background text-foreground shadow-foreground/10 hover:bg-accent pointer-events-auto relative h-12 w-auto max-w-[calc(100vw-1.5rem)] justify-start gap-2.5 rounded-full border px-4 shadow-lg md:min-w-44'
            aria-label={accessibleLabel}
            title={accessibleLabel}
            aria-haspopup='dialog'
            aria-expanded={mobileOpen}
            aria-controls='ai-assistant-panel'
            data-testid='assistant-launcher'
            aria-keyshortcuts='Control+Shift+Space Meta+Shift+Space'
            onPointerEnter={() => void loadAssistantPanel()}
            onFocus={() => void loadAssistantPanel()}
            onClick={() => showAssistant({ id: 'manual', autoSend: false })}
          >
            <HugeiconsIcon
              icon={AiChat02Icon}
              strokeWidth={2}
              className='text-primary size-5'
              data-icon='inline-start'
              aria-hidden='true'
            />
            <span className='relative z-10 truncate text-sm font-medium'>
              {visibleLabel}
            </span>
          </Button>
        </div>
      ) : null}

      {/* Desktop: in-flow right rail. Same stacking level as the main card —
       * opening it shrinks the content area, matching the shell's rounded
       * card language. The wrapper animates width; the inner panel keeps a
       * fixed width so text does not reflow mid-transition. */}
      {!isOverlayViewport ? (
        <div
          className={cn(
            'hidden shrink-0 overflow-hidden transition-[width,margin,opacity] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none xl:block',
            railOpen
              ? cn('opacity-100', RAIL_WIDTH, 'ms-2')
              : 'pointer-events-none w-0 opacity-0'
          )}
          data-testid='assistant-rail'
          data-open={railOpen}
          aria-hidden={!railOpen}
          inert={!railOpen}
        >
          <div className={cn('h-full min-w-0', RAIL_WIDTH)}>
            <Suspense
              fallback={
                <aside
                  className='bg-card h-full w-full rounded-xl border'
                  aria-hidden='true'
                />
              }
            >
              <AssistantPanel
                mode='rail'
                open={railOpen}
                fullscreen={desktopFullscreen}
                initialPreset={initialPreset}
                initialMessage={initialMessage}
                initialMessageRevision={initialMessageRevision}
                autoSendRequestId={autoSendRequestId}
                onAutoSendConsumed={handleAutoSendConsumed}
                onOpenChange={setAssistantRailOpen}
                onConversationReset={handleConversationReset}
                onToggleFullscreen={() =>
                  setDesktop((value) => ({
                    ...value,
                    fullscreen: !value.fullscreen,
                  }))
                }
              />
            </Suspense>
          </div>
        </div>
      ) : null}

      {/* Mobile / narrow overlay sheet. Exclusive with the desktop rail so a
       * queued auto-send can never fire in both presentations. */}
      <Suspense fallback={null}>
        {isOverlayViewport ? (
          <AssistantPanel
            mode='mobile'
            open={mobileOpen}
            initialPreset={initialPreset}
            initialMessage={initialMessage}
            initialMessageRevision={initialMessageRevision}
            autoSendRequestId={autoSendRequestId}
            onAutoSendConsumed={handleAutoSendConsumed}
            onOpenChange={setMobileOpen}
            onConversationReset={handleConversationReset}
          />
        ) : null}
      </Suspense>
    </div>
  )
}
