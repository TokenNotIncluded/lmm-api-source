/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  loginFormSchema,
  registerFormSchema,
  resetPasswordFormSchema,
} from './constants'

test('registration and recovery agree with Core Unicode password boundaries', () => {
  for (const length of [7, 8, 20, 21]) {
    for (const character of ['a', '密', '🔑']) {
      const password = character.repeat(length)
      const input = { username: 'alice', password, confirmPassword: password }
      const expected = length >= 8 && length <= 20
      assert.equal(registerFormSchema.safeParse(input).success, expected)
      assert.equal(resetPasswordFormSchema.safeParse(input).success, expected)
    }
  }
  assert.equal(
    loginFormSchema.safeParse({ username: 'legacy', password: 'old' }).success,
    true
  )
})
