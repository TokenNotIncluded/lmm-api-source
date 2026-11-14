/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { after } from 'node:test'

import { Window } from 'happy-dom'

type LockOptions = { mode?: 'exclusive'; signal?: AbortSignal }
type QueuedRequest = {
  start: () => void
}

/** The exclusive FIFO/abort subset consumed by the browser auth coordinator. */
export function createQueuedLockManager() {
  const names = new Map<string, { active: boolean; queue: QueuedRequest[] }>()
  return {
    request<T>(
      name: string,
      options: LockOptions,
      callback: (lock: {
        name: string
        mode: 'exclusive'
      }) => T | PromiseLike<T>
    ): Promise<T> {
      const { signal } = options
      if (signal?.aborted) {
        return Promise.reject(
          new DOMException('Lock request aborted', 'AbortError')
        )
      }
      let state = names.get(name)
      if (!state) {
        state = { active: false, queue: [] }
        names.set(name, state)
      }
      const lane = state
      const pump = () => {
        if (lane.active) return
        const next = lane.queue.shift()
        if (!next) {
          names.delete(name)
          return
        }
        lane.active = true
        next.start()
      }
      return new Promise<T>((resolve, reject) => {
        let granted = false
        const onAbort = () => {
          if (granted) return
          const index = lane.queue.indexOf(entry)
          if (index === -1) return
          lane.queue.splice(index, 1)
          signal?.removeEventListener('abort', onAbort)
          reject(new DOMException('Lock request aborted', 'AbortError'))
          // An active owner retains exclusion until its callback settles.
          pump()
        }
        const release = () => {
          lane.active = false
          pump()
        }
        const entry: QueuedRequest = {
          start: async () => {
            granted = true
            signal?.removeEventListener('abort', onAbort)
            try {
              resolve(await callback({ name, mode: 'exclusive' }))
            } catch (error: unknown) {
              reject(error)
            } finally {
              release()
            }
          },
        }
        lane.queue.push(entry)
        signal?.addEventListener('abort', onAbort, { once: true })
        queueMicrotask(pump)
      })
    },
  }
}

/** Explicit normal-browser opt-in AFTER a fixture installs window/navigator.
 * Never resets auth modules, uncertainty markers or test-provided storage.
 */
export function installSupportedAuthBrowser(
  options: { window?: object; url?: string } = {}
) {
  const backing = new Window({
    url: options.url ?? 'https://console.example.test/',
  })
  const undo: Array<() => void> = []
  const define = (target: object, key: string, value: unknown) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, key)
    undo.push(() => {
      if (descriptor) Object.defineProperty(target, key, descriptor)
      else Reflect.deleteProperty(target, key)
    })
    Object.defineProperty(target, key, {
      configurable: true,
      writable: true,
      value,
    })
  }
  const browser = options.window ?? Reflect.get(globalThis, 'window') ?? backing
  if (Reflect.get(globalThis, 'window') !== browser) {
    define(globalThis, 'window', browser)
  }
  for (const key of [
    'localStorage',
    'sessionStorage',
    'location',
    'navigator',
  ]) {
    // Test-owned denied storage and other explicit capabilities stay intact.
    if (!(key in browser)) define(browser, key, Reflect.get(backing, key))
  }
  for (const key of [
    'addEventListener',
    'removeEventListener',
    'setTimeout',
    'clearTimeout',
  ]) {
    if (!(key in browser)) {
      define(browser, key, Reflect.get(backing, key).bind(backing))
    }
  }
  const navigator =
    Reflect.get(globalThis, 'navigator') ?? Reflect.get(browser, 'navigator')
  if (Reflect.get(globalThis, 'navigator') !== navigator) {
    define(globalThis, 'navigator', navigator)
  }
  const locks = createQueuedLockManager()
  define(navigator, 'locks', locks)
  const browserNavigator = Reflect.get(browser, 'navigator')
  if (browserNavigator !== navigator) define(browserNavigator, 'locks', locks)
  define(globalThis, 'isSecureContext', true)
  for (const key of ['localStorage', 'sessionStorage']) {
    if (!(key in globalThis)) define(globalThis, key, Reflect.get(browser, key))
  }
  let restored = false
  const restore = () => {
    if (restored) return
    restored = true
    for (const reset of undo.reverse()) reset()
    backing.close()
  }
  after(restore)
  return restore
}
