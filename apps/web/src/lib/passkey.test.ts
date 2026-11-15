/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { isPasskeySupported, requestPasskeyAuthentication } from './passkey.ts'

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  'navigator'
)

function installBrowser(
  secure = true,
  credentials: Record<string, unknown> = {
    create: async () => null,
    get: async () => null,
  }
) {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      isSecureContext: secure,
      PublicKeyCredential: class {
        static async isConditionalMediationAvailable() {
          return false
        }
        static async isUserVerifyingPlatformAuthenticatorAvailable() {
          return false
        }
      },
    },
  })
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { credentials },
  })
}

afterEach(() => {
  for (const [key, descriptor] of [
    ['window', originalWindow],
    ['navigator', originalNavigator],
  ] as const) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('a browser without platform biometrics still supports roaming keys', async () => {
  installBrowser()
  assert.equal(await isPasskeySupported(), true)
})

test('an insecure page cannot offer WebAuthn', async () => {
  installBrowser(false)
  assert.equal(await isPasskeySupported(), false)
})

test('missing WebAuthn methods fail closed', async () => {
  for (const credentials of [{}, { get: async () => null }]) {
    installBrowser(true, credentials)
    assert.equal(await isPasskeySupported(), false)
  }
})

test('server rendering does not advertise browser support', async () => {
  Reflect.deleteProperty(globalThis, 'window')
  assert.equal(await isPasskeySupported(), false)
})

test('Passkey confirmation forwards cancellation and serializes the challenge response', async () => {
  let received: CredentialRequestOptions | undefined
  installBrowser(true, {
    create: async () => null,
    get: async (options: CredentialRequestOptions) => {
      received = options
      return {
        id: 'credential',
        rawId: new Uint8Array([1]).buffer,
        type: 'public-key',
        response: {
          authenticatorData: new Uint8Array([2]).buffer,
          clientDataJSON: new Uint8Array([3]).buffer,
          signature: new Uint8Array([4]).buffer,
          userHandle: null,
        },
      }
    },
  })
  const controller = new AbortController()
  const assertion = await requestPasskeyAuthentication(
    {
      publicKey: { challenge: 'BQ', userVerification: 'required' },
    },
    controller.signal
  )
  assert.equal(received?.signal, controller.signal)
  assert.equal(received?.publicKey?.userVerification, 'required')
  assert.deepEqual(
    new Uint8Array(received?.publicKey?.challenge as ArrayBuffer),
    new Uint8Array([5])
  )
  assert.deepEqual(assertion?.response, {
    authenticatorData: 'Ag',
    clientDataJSON: 'Aw',
    signature: 'BA',
    userHandle: null,
  })
})
