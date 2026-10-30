/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
type Language = 'curl' | 'python' | 'typescript' | 'javascript'

type Context = {
  baseUrl: string
  apiKeyEnv: string
  modelName: string
  endpointType: string
  endpointPath: string
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`
}

/** Each native operation has its own input shape and uses a Core API key. */
export function buildNativeApiSample(
  language: Language,
  context: Context
): string {
  const url = `${context.baseUrl}${context.endpointPath}`
  const payload = (() => {
    switch (context.endpointType) {
      case 'embeddings':
        return { model: context.modelName, input: ['Example document'] }
      case 'jina-rerank':
        return {
          model: context.modelName,
          query: 'How does retrieval work?',
          documents: [
            'Retrieval finds relevant documents.',
            'An unrelated document.',
          ],
          top_n: 1,
        }
      case 'deepseek-search':
        return {
          query: 'Rust documentation site:rust-lang.org',
          count: 5,
          need_content: true,
        }
      case 'deepseek-fetch':
        return { url: 'https://www.rust-lang.org/' }
      default:
        throw new Error('Unsupported native API sample')
    }
  })()
  const body = JSON.stringify(payload, null, 2)
  if (language === 'curl') {
    return [
      `curl ${shellQuote(url)} \\`,
      `  -H "Authorization: Bearer $${context.apiKeyEnv}" \\`,
      '  -H "Content-Type: application/json" \\',
      `  -d ${shellQuote(body)}`,
    ].join('\n')
  }
  if (language === 'python') {
    return [
      'import json',
      'import os',
      'from urllib.request import Request, urlopen',
      '',
      `payload = json.loads(${JSON.stringify(body)})`,
      'request = Request(',
      `    ${JSON.stringify(url)},`,
      '    data=json.dumps(payload).encode("utf-8"),',
      '    headers={',
      `        "Authorization": f"Bearer {os.environ['${context.apiKeyEnv}']}",`,
      '        "Content-Type": "application/json",',
      '    },',
      '    method="POST",',
      ')',
      'with urlopen(request) as response:',
      '    print(json.load(response))',
    ].join('\n')
  }
  return [
    `const response = await fetch(${JSON.stringify(url)}, {`,
    "  method: 'POST',",
    '  headers: {',
    `    Authorization: \`Bearer \${process.env.${context.apiKeyEnv}}\`,`,
    "    'Content-Type': 'application/json',",
    '  },',
    `  body: JSON.stringify(${body}),`,
    '})',
    'if (!response.ok) throw new Error(`HTTP ${response.status}`)',
    'console.log(await response.json())',
  ].join('\n')
}
