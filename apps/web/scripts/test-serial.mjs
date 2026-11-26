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
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/*
Copyright (C) 2026 LIghtJUNction
*/

import { spawn } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const source = join(root, 'src')
const scripts = join(root, 'scripts')
const preload = join(scripts, 'test-preload.mjs')
const sourceTests = []
const scriptTests = []
const failures = []
const jobs = Number(process.env.LMM_WEB_TEST_JOBS ?? '1')

if (!Number.isSafeInteger(jobs) || jobs < 1 || jobs > 8) {
  throw new Error('LMM_WEB_TEST_JOBS must be an integer from 1 to 8')
}

function collect(directory, pattern, target) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const filePath = join(directory, entry.name)
    if (entry.isDirectory()) {
      collect(filePath, pattern, target)
    } else if (pattern.test(entry.name)) {
      target.push(relative(root, filePath))
    }
  }
}

async function run(executable, args) {
  const child = spawn(executable, args, {
    cwd: root,
    stdio: ['ignore', 'inherit', 'inherit'],
  })
  child.on('error', (error) => console.error(error))
  const status = await new Promise((resolve) => child.once('close', resolve))
  if (status !== 0) {
    failures.push({ file: args.at(-1), status: status ?? 1 })
  }
}

collect(source, /\.test\.(?:ts|tsx)$/, sourceTests)
collect(scripts, /\.test\.mjs$/, scriptTests)
sourceTests.sort()
scriptTests.sort()

if (sourceTests.length === 0 && scriptTests.length === 0) {
  throw new Error('no web tests found')
}

const queue = [
  ...sourceTests.map((test) => [
    process.execPath,
    ['test', '--preload', preload, '--timeout', '15000', test],
  ]),
  ...scriptTests.map((test) => ['node', ['--test', test]]),
]

async function runQueue() {
  let task
  while ((task = queue.shift()) !== undefined) {
    await run(...task)
  }
}

await Promise.all(Array.from({ length: jobs }, () => runQueue()))

if (failures.length > 0) {
  console.error('Web test failures:', JSON.stringify(failures, null, 2))
  process.exit(1)
}

console.log(
  `web tests passed: ${sourceTests.length} source + ${scriptTests.length} script files`
)
