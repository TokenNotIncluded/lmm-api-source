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
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: domWindow,
})

const { getPreviewText } = await import('./text')

after(() => domWindow.close())

test('getPreviewText removes nested markup without rebuilding a script tag', () => {
  const preview = getPreviewText('<scr<script>ipt>alert(1)</scr</script>ipt>')

  assert.doesNotMatch(preview, /</)
  assert.match(preview, /&lt;\/script&gt;/)
  assert.match(preview, /alert\(1\)/)
})

test('getPreviewText preserves ordinary text and applies the length limit', () => {
  assert.equal(getPreviewText('<strong>Hello</strong> world', 8), 'Hello wo...')
})
