/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, beforeEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  decodeTrustPolicyJson,
  TrustPolicyEditorClient,
  trustPolicyDraft,
  trustPolicyReadSchema,
} from './trust-policy-editor-client'
import {
  assertionCredential,
  challengeId,
  challengeReply,
  consolePolicy,
  digest,
  envelope,
  policyRead,
  previewReply,
  signIn,
  token,
} from './trust-policy-editor-fixtures'

const get = api.get,
  post = api.post
const navigatorDescriptor = Object.getOwnPropertyDescriptor(
  globalThis,
  'navigator'
)
let calls: { path: string; body: Record<string, unknown>; config: unknown }[] =
  []
let credentialCalls = 0
let credentialGet: (
  options: CredentialRequestOptions
) => Promise<PublicKeyCredential | null>
const client = () =>
  new TrustPolicyEditorClient({ userId: 7, sessionId: 'fixture-session' })
beforeEach(() => {
  signIn()
  calls = []
  credentialCalls = 0
  credentialGet = async (options) => {
    credentialCalls++
    assert.deepEqual(
      new Uint8Array(required(options.publicKey).challenge as ArrayBuffer),
      new Uint8Array([1, 2, 3])
    )
    assert.deepEqual(
      new Uint8Array(
        required(required(options.publicKey).allowCredentials)[0]
          .id as ArrayBuffer
      ),
      new Uint8Array([4, 5, 6])
    )
    assert.equal(required(options.publicKey).rpId, 'console.example.test')
    return assertionCredential()
  }
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: {
      credentials: {
        get: (options: CredentialRequestOptions) => credentialGet(options),
      },
    },
  })
  api.get = (async () => envelope(policyRead())) as typeof api.get
  api.post = (async (
    path: string,
    body: Record<string, unknown>,
    config: unknown
  ) => {
    calls.push({ path, body, config })
    if (path.endsWith('/preview')) {
      return envelope(previewReply(JSON.parse(String(body.value))))
    }
    if (path.endsWith('/challenge')) return envelope(challengeReply())
    if (path.endsWith('/confirm')) return envelope({ revision: 13 })
    if (path.endsWith('/resolve')) {
      return envelope({ state: 'pending', revision: null })
    }
    throw new Error('Unexpected path')
  }) as typeof api.post
})
afterEach(() => {
  api.get = get
  api.post = post
  useAuthStore.getState().auth.reset()
  if (navigatorDescriptor) {
    Object.defineProperty(globalThis, 'navigator', navigatorDescriptor)
  } else Reflect.deleteProperty(globalThis, 'navigator')
})

test('GET preserves numeric authority and null descriptions, without inventing role benefits', async () => {
  const read = await client().read()
  const draft = trustPolicyDraft(read)
  assert.equal(read.console, null)
  assert.equal(draft.tiers[4].min_paid_credits, 400)
  assert.equal(required(draft.role_tiers)[1].discount_ratio, 0.85)
  assert.deepEqual(
    draft.tiers.map((t) => t.benefits),
    [[], [], [], [], []]
  )
  assert.deepEqual(
    required(draft.role_tiers).map((t) => t.benefits),
    [[], []]
  )
  assert.equal(calls.length, 0)
})

test('u64 JSON ratios remain exact; nonrepresentable fractions are read only', () => {
  const wire = JSON.stringify(policyRead()).replace(
    '"numerator":9,"denominator":10',
    '"numerator":900000000000000000,"denominator":1000000000000000000'
  )
  const read = trustPolicyReadSchema.parse(decodeTrustPolicyJson(wire))
  assert.equal(read.policy.admin_discount.numerator, '900000000000000000')
  assert.equal(read.policy.admin_discount.denominator, '1000000000000000000')
  assert.equal(
    required(trustPolicyDraft(read).role_tiers)[0].discount_ratio,
    0.9
  )
  read.policy.admin_discount = { numerator: '1', denominator: '3' }
  assert.throws(() => trustPolicyDraft(read), /exactly/)
  const unsafe = policyRead()
  unsafe.revision = Number.MAX_SAFE_INTEGER + 1
  assert.equal(trustPolicyReadSchema.safeParse(unsafe).success, false)
  assert.throws(() => decodeTrustPolicyJson('{"bad":01}'))
  assert.throws(
    () =>
      decodeTrustPolicyJson(
        String.raw`{"editor_id":"\u005f_trust_policy_integer__0"}`
      ),
    /Invalid/
  )
})

test('rejects malformed GET and descriptions inconsistent with numeric authority', async () => {
  const read = policyRead()
  read.console = consolePolicy()
  read.console.tiers[4].min_paid_credits = 999
  api.get = (async () => envelope(read)) as typeof api.get
  await assert.rejects(client().read(), /exactly/)
  api.get = (async () => ({
    data: { success: false, data: policyRead() },
  })) as typeof api.get
  await assert.rejects(client().read(), /Invalid/)
  const duplicate = policyRead()
  duplicate.editors.push(duplicate.editors[0])
  assert.equal(trustPolicyReadSchema.safeParse(duplicate).success, false)
})

test('preview is immutable; credential challenge occurs only after separate explicit confirm', async () => {
  const edit = client()
  const preview = await edit.preview(consolePolicy())
  assert.equal(credentialCalls, 0)
  assert.deepEqual(
    calls.map((c) => c.path),
    ['/api/option/trust-policy/preview']
  )
  assert.ok(Object.isFrozen(preview.preview.proposed_console.tiers))
  const result = await edit.confirm('owner-enrolled-key')
  assert.deepEqual(result, { state: 'confirmed', revision: 13 })
  assert.equal(credentialCalls, 1)
  assert.deepEqual(calls[1].body, {
    token,
    digest,
    editor: 'owner-enrolled-key',
  })
  assert.deepEqual(calls[2].body, {
    token,
    digest,
    challenge: challengeId,
    assertion: {
      id: 'BAUG',
      rawId: 'BAUG',
      type: 'public-key',
      authenticatorAttachment: 'cross-platform',
      response: {
        authenticatorData: 'Bwg',
        clientDataJSON: 'CQo',
        signature: 'Cww',
        userHandle: null,
      },
      clientExtensionResults: {},
    },
  })
  for (const call of calls) {
    assert.deepEqual((call.config as { authScope: unknown }).authScope, {
      userId: 7,
      sessionId: 'fixture-session',
    })
  }
  await assert.rejects(edit.confirm('owner-enrolled-key'), /new preview/)
  assert.equal(calls.length, 3)
})

test('tampered proposed values, actor, or description cannot reach a challenge', async () => {
  for (const change of [
    (v: ReturnType<typeof previewReply>) => {
      v.preview.actor_user_id = 8
    },
    (v: ReturnType<typeof previewReply>) => {
      v.preview.proposed_console.tiers[4].min_paid_credits = 800
    },
    (v: ReturnType<typeof previewReply>) => {
      v.preview.proposed.admin_discount.numerator = 8
    },
    (v: ReturnType<typeof previewReply>) => {
      v.preview.current_console = consolePolicy()
      v.preview.current_console.decay_period_days = 7
    },
  ]) {
    const reply = previewReply()
    change(reply)
    api.post = (async () => envelope(reply)) as typeof api.post
    const edit = client()
    await assert.rejects(edit.preview(consolePolicy()))
    await assert.rejects(edit.confirm('owner-enrolled-key'))
  }
  assert.equal(credentialCalls, 0)
})

for (const failure of [
  'cancel',
  'lost challenge',
  'lost confirm',
  'bad confirm',
]) {
  test(`${failure} resolves only the original token; never repeats confirmation`, async () => {
    const standard = api.post
    api.post = (async (
      path: string,
      body: Record<string, unknown>,
      config: unknown
    ) => {
      if (
        (failure === 'lost challenge' && path.endsWith('/challenge')) ||
        (failure === 'lost confirm' && path.endsWith('/confirm'))
      ) {
        calls.push({ path, body, config })
        throw new Error('response lost')
      }
      if (failure === 'bad confirm' && path.endsWith('/confirm')) {
        calls.push({ path, body, config })
        return envelope({ revision: 99 })
      }
      if (path.endsWith('/resolve')) {
        calls.push({ path, body, config })
        return envelope({
          state: failure === 'lost confirm' ? 'confirmed' : 'pending',
          revision: failure === 'lost confirm' ? 13 : null,
        })
      }
      return standard(path, body, config as never)
    }) as typeof api.post
    if (failure === 'cancel') {
      credentialGet = async () => {
        throw new DOMException('Cancelled', 'NotAllowedError')
      }
    }
    const edit = client()
    await edit.preview(consolePolicy())
    const result = await edit.confirm('owner-enrolled-key')
    assert.equal(
      result.state,
      failure === 'lost confirm' ? 'confirmed' : 'pending'
    )
    assert.deepEqual(required(calls.at(-1)).body, { token })
    assert.equal(
      calls.filter((c) => c.path.endsWith('/confirm')).length,
      failure.endsWith('confirm') ? 1 : 0
    )
    await assert.rejects(edit.confirm('owner-enrolled-key'), /new preview/)
    assert.equal(edit.canDiscard, failure !== 'bad confirm')
  })
}

test('unresolved commit keeps token, prohibits discarding; later resolve can finish', async () => {
  const standard = api.post
  let resolves = 0
  api.post = (async (path: string, body: unknown, config: unknown) => {
    if (path.endsWith('/confirm')) throw new Error('lost')
    if (path.endsWith('/resolve')) {
      assert.deepEqual(body, { token })
      if (++resolves === 1) throw new Error('offline')
      return envelope({ state: 'confirmed', revision: 13 })
    }
    return standard(path, body, config as never)
  }) as typeof api.post
  const edit = client()
  await edit.preview(consolePolicy())
  await assert.rejects(edit.confirm('owner-enrolled-key'))
  assert.equal(edit.canDiscard, false)
  assert.throws(() => edit.discard(), /original review/)
  assert.deepEqual(await edit.resolve(), { state: 'confirmed', revision: 13 })
  assert.equal(edit.canDiscard, true)
})

test('renewed same actor session resolves original token but cannot confirm the old preview', async () => {
  const edit = client()
  await edit.preview(consolePolicy())
  signIn(7, 'renewed')
  await assert.rejects(edit.confirm('owner-enrolled-key'), /session changed/)
  assert.deepEqual(await edit.resolve(), { state: 'pending', revision: null })
  assert.deepEqual(required(calls.at(-1)).body, { token })
  assert.deepEqual(
    (required(calls.at(-1)).config as { authScope: unknown }).authScope,
    {
      userId: 7,
      sessionId: 'renewed',
    }
  )
  signIn(8, 'other-account')
  const count = calls.length
  await assert.rejects(edit.resolve(), /session changed/)
  assert.equal(calls.length, count)
})

test('account switch during navigator prompt never sends assertion to confirm or resolve', async () => {
  credentialGet = async () => {
    signIn(8, 'other-account')
    return assertionCredential()
  }
  const edit = client()
  await edit.preview(consolePolicy())
  await assert.rejects(edit.confirm('owner-enrolled-key'), /session changed/)
  assert.equal(calls.length, 2)
})

test('unmount aborts credential flow and rejects late GET responses', async () => {
  const edit = client()
  api.get = (async () => {
    edit.dispose()
    return envelope(policyRead())
  }) as typeof api.get
  await assert.rejects(edit.read(), /session changed/)
  const next = client()
  await next.preview(consolePolicy())
  credentialGet = async (options) => {
    next.dispose()
    assert.equal(required(options.signal).aborted, true)
    return assertionCredential()
  }
  await assert.rejects(next.confirm('owner-enrolled-key'))
  assert.equal(calls.filter((c) => c.path.endsWith('/confirm')).length, 0)
})

test('expired and unknown are not success and permit a fresh review', async () => {
  for (const state of ['expired', 'unknown']) {
    const edit = client()
    await edit.preview(consolePolicy())
    const previous = api.post
    api.post = (async (path: string, body: unknown, config: unknown) =>
      path.endsWith('/resolve')
        ? envelope({ state, revision: null })
        : previous(path, body, config as never)) as typeof api.post
    assert.deepEqual(await edit.resolve(), { state, revision: null })
    edit.discard()
    await edit.preview(consolePolicy())
  }
})

function required<T>(value: T | null | undefined): T {
  assert.ok(value !== null && value !== undefined)
  return value
}
