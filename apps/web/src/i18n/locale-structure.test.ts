/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const locales = ['en', 'zh-CN', 'zh-TW', 'fr', 'ja', 'ru', 'vi'] as const
const testDirectory = path.dirname(fileURLToPath(import.meta.url))
const subscriptionDeleteError =
  'Subscription plan has subscription or order history and cannot be deleted. Disable it instead.'

test('locale files keep every UI key inside the translation namespace', async () => {
  for (const locale of locales) {
    const file = path.join(testDirectory, 'locales', `${locale}.json`)
    const json = JSON.parse(await fs.readFile(file, 'utf8')) as Record<
      string,
      unknown
    >
    assert.deepEqual(Object.keys(json), ['translation'], locale)

    const translation = json.translation as Record<string, string>
    assert.equal(typeof translation[subscriptionDeleteError], 'string', locale)
    assert.ok(translation[subscriptionDeleteError].length > 0, locale)
    if (locale !== 'en') {
      assert.notEqual(
        translation[subscriptionDeleteError],
        subscriptionDeleteError,
        `${locale} must translate the subscription deletion guard`
      )
    }
  }
})

test('dynamic auth-cookie recovery and storage messages remain extractable and translated', async () => {
  const source = await fs.readFile(
    new URL('../lib/auth-cookie.ts', import.meta.url),
    'utf8'
  )
  const { STATIC_I18N_KEYS } = await import('./static-keys')
  const staticKeys = new Set<string>(STATIC_I18N_KEYS)
  for (const name of [
    'authCookieRecoveryMessage',
    'authCookieStorageMessage',
  ]) {
    const match = source.match(
      new RegExp(`export const ${name} =\\n  '([^']+)'`)
    )
    assert.ok(match)
    const key = match[1]
    assert.ok(staticKeys.has(key))
    for (const locale of locales) {
      const document = JSON.parse(
        await fs.readFile(
          new URL(`./locales/${locale}.json`, import.meta.url),
          'utf8'
        )
      )
      assert.ok(document.translation[key], `${locale}: ${name}`)
      if (locale !== 'en') {
        assert.notEqual(document.translation[key], key, `${locale}: ${name}`)
      }
    }
  }
})

test('OAuth grant client categories have real translations in all interface languages', async () => {
  for (const locale of locales) {
    const document = JSON.parse(
      await fs.readFile(
        new URL(`./locales/${locale}.json`, import.meta.url),
        'utf8'
      )
    )
    for (const key of ['Model client', 'Read-only client']) {
      assert.ok(document.translation[key], `${locale}: ${key}`)
      if (locale !== 'en') {
        assert.notEqual(document.translation[key], key, `${locale}: ${key}`)
      }
    }
  }
})
