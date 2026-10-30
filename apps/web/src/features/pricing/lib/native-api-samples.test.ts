/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildNativeApiSample } from './native-api-samples'

test('native samples send the real operation shape in all supported languages', () => {
  for (const language of [
    'curl',
    'python',
    'typescript',
    'javascript',
  ] as const) {
    for (const [endpointType, endpointPath, fields] of [
      ['embeddings', '/v1/embeddings', ['model', 'input']],
      ['jina-rerank', '/v1/rerank', ['model', 'query', 'documents', 'top_n']],
      [
        'deepseek-search',
        '/v1/tools/search',
        ['query', 'count', 'need_content'],
      ],
      ['deepseek-fetch', '/v1/tools/fetch', ['url']],
    ] as const) {
      const sample = buildNativeApiSample(language, {
        baseUrl: 'https://gateway.invalid',
        apiKeyEnv: 'LMM_API_KEY',
        modelName: 'configured-model',
        endpointType,
        endpointPath,
      })
      assert.ok(sample.includes(`https://gateway.invalid${endpointPath}`))
      assert.ok(sample.includes('LMM_API_KEY'))
      for (const field of fields) {
        assert.ok(sample.includes(field), `${language}: ${field}`)
      }
      assert.ok(!sample.includes('messages'))
      if (endpointType.startsWith('deepseek-')) {
        assert.ok(!sample.includes('configured-model'))
      }
      if (language === 'python') {
        assert.ok(sample.includes('payload = json.loads('))
      }
    }
  }
})
