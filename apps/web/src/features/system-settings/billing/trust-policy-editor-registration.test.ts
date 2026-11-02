/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import {
  createPublicRegistration,
  parsePublicRegistration,
  REGISTRATION_MAX_BYTES,
} from './trust-policy-editor-registration-contract'

const origin = 'http://localhost:49173'
function fixture() {
  return {
    protocol: 'core.trust-policy-owner.v1',
    status: 'confirmed',
    data: {
      enrollment: 'ab'.repeat(32),
      options: {
        publicKey: {
          rp: { id: 'localhost', name: 'LMM policy editor' },
          user: {
            id: 'BAUG',
            name: 'synthetic-editor',
            displayName: 'synthetic-editor',
          },
          challenge: 'AQID',
          pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
          timeout: 300000,
          attestation: 'none',
          authenticatorSelection: {
            requireResidentKey: false,
            residentKey: 'discouraged',
            userVerification: 'required',
          },
          extensions: {
            credProps: true,
            credentialProtectionPolicy: 'userVerificationRequired',
            enforceCredentialProtectionPolicy: false,
            uvm: true,
          },
        },
      },
    },
  }
}
const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'window')
const navigatorDescriptor = Object.getOwnPropertyDescriptor(
  globalThis,
  'navigator'
)
afterEach(() => {
  for (const [key, descriptor] of [
    ['window', windowDescriptor],
    ['navigator', navigatorDescriptor],
  ] as const) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})
function browser(
  create: (input: CredentialCreationOptions) => Promise<Credential | null>,
  secure = true
) {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { isSecureContext: secure, location: { origin } },
  })
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { credentials: { create } },
  })
}

test('accepts exact CLI begin envelope, but does not infer actor ID or approval from confirmed', () => {
  const value = parsePublicRegistration(JSON.stringify(fixture()), origin)
  assert.equal(value.enrollment, 'ab'.repeat(32))
  assert.equal(value.options.publicKey.user.name, 'synthetic-editor')
  assert.equal('actor_user_id' in value, false)
  assert.equal('status' in value, false)
  assert.equal('approved' in value, false)
})

test('accepts HTTPS and exact localhost RP; rejects HTTP remote, RP suffix and noncanonical origins', () => {
  const raw = JSON.stringify(fixture())
  assert.doesNotThrow(() => parsePublicRegistration(raw, 'https://localhost'))
  for (const url of [
    'http://127.0.0.1:49173',
    'http://localhost.evil:49173',
    'http://example.test',
    `${origin}/`,
    `${origin}/path`,
  ]) {
    assert.throws(() => parsePublicRegistration(raw, url))
  }
  const production = fixture()
  production.data.options.publicKey.rp.id = 'console.example.test'
  assert.doesNotThrow(() =>
    parsePublicRegistration(
      JSON.stringify(production),
      'https://console.example.test'
    )
  )
})

test('rejects private or undeclared input fields and unrelated owner receipts', () => {
  const value = fixture()
  for (const bad of [
    { ...value, secret: 'synthetic' },
    { ...value, status: 'observed' },
    { ...value, data: { version: 1 } },
    {
      ...value,
      data: { ...value.data, registration: { private_state: 'synthetic' } },
    },
    {
      ...value,
      data: {
        ...value.data,
        options: { ...value.data.options, signing_key: 'synthetic' },
      },
    },
  ]) {
    assert.throws(() => parsePublicRegistration(JSON.stringify(bad), origin))
  }
  const bad = fixture()
  Object.assign(bad.data.options.publicKey.extensions, { secret: 'synthetic' })
  assert.throws(() => parsePublicRegistration(JSON.stringify(bad), origin))
})

test('rejects oversized, malformed, missing and weakened creation options', () => {
  assert.throws(() =>
    parsePublicRegistration(' '.repeat(REGISTRATION_MAX_BYTES + 1), origin)
  )
  assert.throws(() => parsePublicRegistration('{', origin))
  assert.throws(() => parsePublicRegistration('{}', origin))
  const bad = fixture()
  bad.data.options.publicKey.authenticatorSelection.userVerification =
    'discouraged'
  assert.throws(() => parsePublicRegistration(JSON.stringify(bad), origin))
})

test('existing converter passes binary challenge/user ID to create and exports only public registration response', async () => {
  const signal = new AbortController().signal
  browser(async (input) => {
    assert.equal(input.signal, signal)
    assert.ok(input.publicKey)
    assert.deepEqual(
      new Uint8Array(input.publicKey.challenge as ArrayBuffer),
      new Uint8Array([1, 2, 3])
    )
    assert.deepEqual(
      new Uint8Array(input.publicKey.user.id as ArrayBuffer),
      new Uint8Array([4, 5, 6])
    )
    return {
      type: 'public-key',
      id: 'BAUG',
      rawId: new Uint8Array([4, 5, 6]).buffer,
      response: {
        attestationObject: new Uint8Array([7, 8]).buffer,
        clientDataJSON: new Uint8Array([9, 10]).buffer,
        getTransports: () => ['usb'],
      },
      getClientExtensionResults: () => ({}),
    } as unknown as PublicKeyCredential
  })
  const response = JSON.parse(
    await createPublicRegistration(
      parsePublicRegistration(JSON.stringify(fixture()), origin),
      signal
    )
  )
  assert.deepEqual(response, {
    id: 'BAUG',
    rawId: 'BAUG',
    type: 'public-key',
    response: {
      attestationObject: 'Bwg',
      clientDataJSON: 'CQo',
      transports: ['usb'],
    },
    clientExtensionResults: {},
  })
  assert.equal('enrollment' in response, false)
  assert.equal('operation' in response, false)
  assert.equal('approved' in response, false)
})

test('cancellation and insecure context export no response', async () => {
  const value = parsePublicRegistration(JSON.stringify(fixture()), origin)
  browser(async () => null)
  await assert.rejects(
    createPublicRegistration(value, new AbortController().signal),
    /cancelled/
  )
  browser(async () => {
    throw new DOMException('Cancelled', 'NotAllowedError')
  })
  await assert.rejects(
    createPublicRegistration(value, new AbortController().signal)
  )
  browser(async () => {
    assert.fail('must not prompt in insecure context')
  }, false)
  await assert.rejects(
    createPublicRegistration(value, new AbortController().signal),
    /Secure/
  )
})

test('discard a late browser result after cancellation', async () => {
  const controller = new AbortController()
  browser(async () => {
    controller.abort()
    return null
  })
  await assert.rejects(
    createPublicRegistration(
      parsePublicRegistration(JSON.stringify(fixture()), origin),
      controller.signal
    ),
    /cancelled/
  )
})

test('refuse public response that cannot fit the real CLI review envelope', async () => {
  browser(
    async () =>
      ({
        type: 'public-key',
        id: 'AQID',
        rawId: new Uint8Array([1, 2, 3]).buffer,
        response: {
          attestationObject: new Uint8Array(85000).buffer,
          clientDataJSON: new Uint8Array([1]).buffer,
          getTransports: () => ['usb'],
        },
        getClientExtensionResults: () => ({}),
      }) as unknown as PublicKeyCredential
  )
  await assert.rejects(
    createPublicRegistration(
      parsePublicRegistration(JSON.stringify(fixture()), origin),
      new AbortController().signal
    ),
    /CLI limit/
  )
})
