// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { GoogleGenAI } from '@google/genai'

const { origin, key } = JSON.parse(readFileSync(0, 'utf8'))
assert.equal(new URL(origin).hostname, '127.0.0.1')
assert.equal(new URL(origin).protocol, 'http:')
const genai = new GoogleGenAI({
  apiKey: key,
  httpOptions: { baseUrl: origin, apiVersion: 'v1beta', timeout: 10000 },
})
const chat = genai.chats.create({
  model: 'native-model',
  config: { maxOutputTokens: 20 },
})
let visible = ''
let last
let stops = 0
for await (const chunk of await chat.sendMessageStream({ message: 'hello' })) {
  visible += chunk.text ?? ''
  if (chunk.candidates?.[0]?.finishReason === 'STOP') stops++
  last = chunk
}
assert.equal(visible, 'native answer')
assert.equal(stops, 1)
assert.equal(last?.candidates?.[0]?.finishReason, 'STOP')
assert.equal(last?.usageMetadata?.promptTokenCount, 10)
assert.equal(last?.usageMetadata?.totalTokenCount, 30)
const history = chat.getHistory(true)
assert.equal(history[0]?.role, 'user')
assert.equal(history[0]?.parts?.[0]?.text, 'hello')
const retained = history.slice(1).flatMap(content => {
  assert.equal(content.role, 'model')
  return content.parts.map(part => part.text)
})
assert.equal(retained.join(''), visible)
