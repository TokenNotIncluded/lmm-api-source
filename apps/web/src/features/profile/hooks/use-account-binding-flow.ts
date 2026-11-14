/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { useCallback, useEffect, useReducer, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import {
  OAUTH_BIND_CALLBACK_MESSAGE,
  OAUTH_BIND_RESULT_MESSAGE,
} from '@/features/auth/constants'
import {
  reportAuthError,
  shouldSuppressAuthError,
} from '@/features/auth/lib/auth-completion'
import {
  federatedProviders,
  forgetFederatedCeremony,
  saveFederatedCeremony,
} from '@/features/auth/lib/federated-oauth'
import { watchOAuthPopupClosed } from '@/features/auth/lib/oauth-bind-window'
import {
  getOAuthSessionStorage,
  markOAuthBindPopup,
} from '@/features/auth/lib/oauth-callback-mode'
import type { SystemStatus } from '@/features/auth/types'
import { useAuthStore } from '@/stores/auth-store'

import {
  accountLinkScope,
  assertAccountLinkScope,
  beginAccountBinding,
  finishAccountBinding,
  readAccountLinks,
  unlinkAccount,
  type AccountLink,
  type AccountLinks,
  type AccountLinkScope,
  type AccountProvider,
  type AccountProviderDescriptor,
} from '../account-links-api'

type Operation = Readonly<{
  id: number
  generation: number
  scope: AccountLinkScope
}>
type BindOperation = Operation &
  Readonly<{
    kind: 'bind'
    descriptor: AccountProviderDescriptor
    provider: AccountProvider
    slug: string
  }>
type UnlinkOperation = Operation &
  Readonly<{ kind: 'unlink'; target: AccountLink }>
type CodeOperation = Readonly<{
  descriptor: AccountProviderDescriptor
  state: string
}>
type AccountBindingPhase =
  | Readonly<{ tag: 'idle' }>
  | Readonly<{ tag: 'starting' | 'awaitingPopup'; operationId: number }>
  | Readonly<{
      tag: 'awaitingCode' | 'finishing'
      operationId: number
      codeOperation: CodeOperation | null
    }>
type Owner = Readonly<{ user: number | undefined; sid: string | undefined }>
type State = Readonly<{
  owner: Owner
  phase: AccountBindingPhase
  view: AccountLinks | null
  viewScope: AccountLinkScope | null
  failed: boolean
  target: AccountLink | null
}>
type OperationRuntime = {
  operation: BindOperation | UnlinkOperation
  // Fences callbacks locally; dispatched cookie writes still settle in the API.
  abort: AbortController
  finishing: boolean
  popup: Window | null
  state: string
  stop: () => void
}
type Action =
  | { type: 'reset'; owner: Owner }
  | { type: 'loading' }
  | { type: 'loaded'; view: AccountLinks; scope: AccountLinkScope }
  | { type: 'loadFailed' }
  | { type: 'select'; target: AccountLink | null }
  | { type: 'transition'; phase: Exclude<AccountBindingPhase, { tag: 'idle' }> }
  | { type: 'settle'; id: number; clearTarget?: boolean }
const initial: State = {
  owner: { user: undefined, sid: undefined },
  phase: { tag: 'idle' },
  view: null,
  viewScope: null,
  failed: false,
  target: null,
}
function reduce(state: State, action: Action): State {
  switch (action.type) {
    case 'reset':
      return { ...initial, owner: action.owner }
    case 'loading':
      return { ...state, failed: false }
    case 'loaded':
      return {
        ...state,
        view: action.view,
        viewScope: action.scope,
        failed: false,
      }
    case 'loadFailed':
      return { ...state, view: null, viewScope: null, failed: true }
    case 'select':
      return state.phase.tag === 'idle'
        ? { ...state, target: action.target }
        : state
    case 'transition':
      return state.phase.tag === 'idle' ||
        state.phase.operationId === action.phase.operationId
        ? { ...state, phase: action.phase }
        : state
    case 'settle':
      return state.phase.tag !== 'idle' && state.phase.operationId === action.id
        ? {
            ...state,
            phase: { tag: 'idle' },
            target: action.clearTarget ? null : state.target,
          }
        : state
  }
}
function scopeCurrent(scope: AccountLinkScope) {
  const auth = useAuthStore.getState().auth
  return auth.user?.id === scope.userId && auth.session?.sid === scope.sessionId
}
function stopPopup(resource: OperationRuntime) {
  const stop = resource.stop
  resource.stop = () => {}
  stop()
}
function dispose(resource: OperationRuntime) {
  resource.abort.abort()
  stopPopup(resource)
  if (resource.state.startsWith('lmmfe_')) {
    forgetFederatedCeremony(window.sessionStorage, resource.state)
  }
  if (resource.popup && !resource.popup.closed) resource.popup.close()
}
function codeOperation(resource: OperationRuntime): CodeOperation | null {
  const operation = resource.operation
  return operation.kind === 'bind' &&
    'native_federated' in operation.descriptor &&
    operation.descriptor.kind === 'wechat'
    ? { descriptor: operation.descriptor, state: resource.state }
    : null
}
export function useAccountBindingFlow(
  status: SystemStatus,
  onUpdate: () => void | Promise<void>
) {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user?.id)
  const sid = useAuthStore((state) => state.auth.session?.sid)
  const [state, dispatch] = useReducer(reduce, {
    ...initial,
    owner: { user, sid },
  })
  const runtime = useRef({
    generation: 0,
    nextId: 0,
    read: 0,
    active: null as OperationRuntime | null,
  })
  if (state.owner.user !== user || state.owner.sid !== sid) {
    dispatch({ type: 'reset', owner: { user, sid } })
  }
  const owns = useCallback((resource: OperationRuntime) => {
    const operation = resource.operation
    return (
      !resource.abort.signal.aborted &&
      runtime.current.generation === operation.generation &&
      runtime.current.active?.operation.id === operation.id &&
      scopeCurrent(operation.scope)
    )
  }, [])
  const load = useCallback(async (): Promise<
    'loaded' | 'failed' | 'superseded'
  > => {
    const generation = runtime.current.generation,
      read = ++runtime.current.read
    try {
      const scope = accountLinkScope(),
        view = await readAccountLinks(scope)
      if (
        generation === runtime.current.generation &&
        read === runtime.current.read
      ) {
        dispatch({ type: 'loaded', view, scope })
        return 'loaded'
      }
    } catch (error) {
      if (
        generation === runtime.current.generation &&
        read === runtime.current.read &&
        !shouldSuppressAuthError(error)
      ) {
        dispatch({ type: 'loadFailed' })
        return 'failed'
      }
    }
    return 'superseded'
  }, [])
  useEffect(() => {
    const lifecycle = runtime.current
    void load()
    return () => {
      lifecycle.generation++
      const resource = lifecycle.active
      lifecycle.active = null
      if (resource) dispose(resource)
    }
  }, [user, sid, load])
  const cancel = useCallback(() => {
    const resource = runtime.current.active
    if (!resource || resource.finishing) return
    runtime.current.active = null
    dispose(resource)
    dispatch({ type: 'settle', id: resource.operation.id })
  }, [])
  const begin = async (descriptor: AccountProviderDescriptor) => {
    const snapshot = state
    if (runtime.current.active || !snapshot.view || !snapshot.viewScope) return
    if (!scopeCurrent(snapshot.viewScope)) return
    const native = 'native_federated' in descriptor,
      isWechat = native && descriptor.kind === 'wechat'
    const popup = isWechat ? null : window.open('', '_blank')
    if (!isWechat && !popup) {
      toast.error(t('OAuth pop-up was blocked'))
      return
    }
    const operation: BindOperation = {
      id: ++runtime.current.nextId,
      generation: runtime.current.generation,
      kind: 'bind',
      scope: snapshot.viewScope,
      descriptor,
      provider: descriptor.provider as AccountProvider,
      slug: native ? descriptor.slug : descriptor.provider,
    }
    const resource: OperationRuntime = {
      operation,
      abort: new AbortController(),
      finishing: false,
      popup,
      state: '',
      stop: () => {},
    }
    runtime.current.active = resource
    dispatch({
      type: 'transition',
      phase: { tag: 'starting', operationId: operation.id },
    })
    try {
      const started = await beginAccountBinding(
        operation.scope,
        operation.provider,
        snapshot.view.expected_auth_version,
        status,
        window.location.origin
      )
      resource.state = started.state
      if (!owns(resource)) {
        dispose(resource)
        return
      }
      if (popup?.closed) {
        cancel()
        return
      }
      assertAccountLinkScope(operation.scope)
      if (isWechat) {
        dispatch({
          type: 'transition',
          phase: {
            tag: 'awaitingCode',
            operationId: operation.id,
            codeOperation: codeOperation(resource),
          },
        })
        return
      }
      if (!popup) throw new Error('OAuth pop-up was blocked')
      if (native) {
        const configured = federatedProviders(status).find(
          (row) => row.id === descriptor.provider_id
        )
        if (!configured) throw new Error('Account binding failed')
        saveFederatedCeremony(popup.sessionStorage, configured, started, 'bind')
      }
      if (
        !markOAuthBindPopup(
          getOAuthSessionStorage(popup),
          operation.slug,
          started.state
        )
      ) {
        throw new Error('Popup storage unavailable')
      }
      resource.stop = watchOAuthPopupClosed(popup, () => {
        if (owns(resource)) cancel()
      })
      dispatch({
        type: 'transition',
        phase: { tag: 'awaitingPopup', operationId: operation.id },
      })
      popup.location.replace(started.authorization_url)
    } catch (error) {
      if (!owns(resource)) return
      runtime.current.active = null
      dispose(resource)
      dispatch({ type: 'settle', id: operation.id })
      reportAuthError(
        error,
        t,
        t(
          'Account binding failed. Sign in again if your session is older than 10 minutes.'
        )
      )
    }
  }
  const finish = useCallback(
    async (
      resource: OperationRuntime,
      input: { code: string; iss?: string } | null,
      notify: boolean
    ) => {
      const operation = resource.operation
      if (
        operation.kind !== 'bind' ||
        !owns(resource) ||
        resource.finishing ||
        !resource.state
      ) {
        throw new axios.CanceledError('Binding superseded')
      }
      resource.finishing = true
      stopPopup(resource)
      dispatch({
        type: 'transition',
        phase: {
          tag: 'finishing',
          operationId: operation.id,
          codeOperation: codeOperation(resource),
        },
      })
      let success = false
      try {
        if (!input) throw new Error('Provider rejected binding')
        await finishAccountBinding(operation.scope, operation.provider, {
          state: resource.state,
          ...input,
        })
        if (!owns(resource)) {
          throw new axios.CanceledError('Binding superseded')
        }
        assertAccountLinkScope(operation.scope)
        success = true
        if (notify) toast.success(t('Binding successful!'))
        await load()
        if (owns(resource)) void onUpdate()
      } catch (error) {
        if (!owns(resource)) throw new axios.CanceledError('Binding superseded')
        if (!shouldSuppressAuthError(error)) {
          if (notify) {
            reportAuthError(
              error,
              t,
              t(
                'Account binding could not be confirmed. Refresh the list before trying again.'
              )
            )
          }
          void load()
        }
        throw error
      } finally {
        const active = owns(resource)
        if (resource.state.startsWith('lmmfe_')) {
          forgetFederatedCeremony(window.sessionStorage, resource.state)
        }
        if (active) {
          runtime.current.active = null
          resource.abort.abort()
          dispatch({ type: 'settle', id: operation.id })
        }
        if (active && resource.popup && !resource.popup.closed) {
          resource.popup.postMessage(
            {
              type: OAUTH_BIND_RESULT_MESSAGE,
              provider: operation.slug,
              state: resource.state,
              success,
              message: success ? undefined : t('Account binding failed'),
            },
            window.location.origin
          )
        }
      }
    },
    [load, onUpdate, owns, t]
  )
  useEffect(() => {
    const callback = (event: MessageEvent<unknown>) => {
      const resource = runtime.current.active
      if (
        !resource ||
        !resource.popup ||
        resource.operation.kind !== 'bind' ||
        resource.finishing ||
        !resource.state
      ) {
        return
      }
      const operation = resource.operation,
        message = event.data as {
          type?: unknown
          provider?: unknown
          state?: unknown
          code?: unknown
          iss?: unknown
          error?: unknown
        } | null
      if (
        event.origin !== window.location.origin ||
        event.source !== resource.popup ||
        !message ||
        message.type !== OAUTH_BIND_CALLBACK_MESSAGE ||
        message.provider !== operation.slug ||
        message.state !== resource.state
      ) {
        return
      }
      const input =
        message.error ||
        typeof message.code !== 'string' ||
        (message.iss !== undefined && typeof message.iss !== 'string')
          ? null
          : { code: message.code, iss: message.iss as string | undefined }
      void finish(resource, input, true).catch(() => undefined)
    }
    window.addEventListener('message', callback)
    return () => window.removeEventListener('message', callback)
  }, [finish])
  const submitCode = async (code: string) => {
    const resource = runtime.current.active
    if (!resource || !codeOperation(resource)) {
      throw new axios.CanceledError('Binding superseded')
    }
    await finish(resource, { code }, false)
  }
  const unlink = async () => {
    const snapshot = state
    if (
      runtime.current.active ||
      !snapshot.target ||
      !snapshot.view ||
      !snapshot.viewScope
    ) {
      return
    }
    if (!scopeCurrent(snapshot.viewScope)) return
    const operation: UnlinkOperation = {
      id: ++runtime.current.nextId,
      generation: runtime.current.generation,
      kind: 'unlink',
      scope: snapshot.viewScope,
      target: snapshot.target,
    }
    const resource: OperationRuntime = {
      operation,
      abort: new AbortController(),
      finishing: true,
      popup: null,
      state: '',
      stop: () => {},
    }
    runtime.current.active = resource
    dispatch({
      type: 'transition',
      phase: {
        tag: 'finishing',
        operationId: operation.id,
        codeOperation: null,
      },
    })
    try {
      await unlinkAccount(
        operation.scope,
        operation.target,
        snapshot.view.expected_auth_version
      )
      if (!owns(resource)) return
      toast.success(
        t('Unbound {{provider}}', { provider: operation.target.issuer })
      )
      await load()
      if (owns(resource)) void onUpdate()
      dispatch({ type: 'settle', id: operation.id, clearTarget: true })
    } catch (error) {
      if (!owns(resource)) return
      if (!shouldSuppressAuthError(error)) {
        reportAuthError(
          error,
          t,
          t(
            'Account binding could not be confirmed. Refresh the list before trying again.'
          )
        )
        void load()
      }
    } finally {
      if (owns(resource)) {
        runtime.current.active = null
        resource.abort.abort()
        dispatch({ type: 'settle', id: operation.id })
      }
    }
  }
  return {
    state,
    busy: state.phase.tag !== 'idle',
    codeOperation:
      state.phase.tag === 'awaitingCode' || state.phase.tag === 'finishing'
        ? state.phase.codeOperation
        : null,
    load: () => {
      dispatch({ type: 'loading' })
      return load()
    },
    begin,
    cancel,
    submitCode,
    unlink,
    selectTarget: (target: AccountLink | null) =>
      dispatch({ type: 'select', target }),
    onCodeDialogChange: (open: boolean) => {
      if (!open) cancel()
    },
  }
}
