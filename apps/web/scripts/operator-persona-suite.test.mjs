#!/usr/bin/env node
/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import { after, test } from 'node:test'
import { fileURLToPath } from 'node:url'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDirectory, 'operator-persona-suite.mjs')
const repository = path.resolve(scriptDirectory, '../../..')
// The suite deliberately rejects /tmp deployment roots. Test the missing
// marker in a private, canonical home directory, independent of checkout path.
const workspace = realpathSync(
  mkdtempSync(path.join(homedir(), '.lmm-persona-test-'))
)
const outputDirectory = path.join(workspace, 'evidence')
mkdirSync(outputDirectory, { mode: 0o700 })
after(() => rmSync(workspace, { recursive: true, force: true }))

function run(environment) {
  return spawnSync(process.execPath, [script], {
    cwd: repository,
    env: {
      ...process.env,
      PERSONA_REVIEW_URL: 'http://127.0.0.1:4174',
      PERSONA_DEPLOY_WORKSPACE: workspace,
      PERSONA_OUTPUT_DIR: outputDirectory,
      ...environment,
    },
    encoding: 'utf8',
  })
}

test('rejects a production origin before reading workspace credentials', () => {
  const result = run({ PERSONA_REVIEW_URL: 'https://api.lmm.best' })
  assert.notEqual(result.status, 0)
  assert.match(
    `${result.stdout}\n${result.stderr}`,
    /PERSONA_REVIEW_URL must be exactly http:\/\/127\.0\.0\.1:4174/
  )
})

test('rejects a local run without a marker-owned deployment workspace', () => {
  const result = run({})
  assert.notEqual(result.status, 0)
  assert.match(
    `${result.stdout}\n${result.stderr}`,
    /deployment marker is missing|deployment workspace marker is missing|ENOENT: no such file or directory/
  )
})

test('rejects an alternate local port', () => {
  const result = run({ PERSONA_REVIEW_URL: 'http://127.0.0.1:4175' })
  assert.notEqual(result.status, 0)
  assert.match(
    `${result.stdout}\n${result.stderr}`,
    /PERSONA_REVIEW_URL must be exactly http:\/\/127\.0\.0\.1:4174/
  )
})

// Keep the unsafe-directory rejection separate from the missing-marker case.
test('rejects an output directory that is the deployment workspace itself', () => {
  const result = run({ PERSONA_OUTPUT_DIR: workspace })
  assert.notEqual(result.status, 0)
  assert.match(
    `${result.stdout}\n${result.stderr}`,
    /workspace or output directory is unsafe/
  )
})
