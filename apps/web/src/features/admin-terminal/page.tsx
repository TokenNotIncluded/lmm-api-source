/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import {
  readNativeRoutingScope,
  type NativeRoutingScope,
} from '@/features/channels/native-api'
import { useAuthStore } from '@/stores/auth-store'

import { terminalError, type CatalogContext } from './catalog'
import {
  executeCommand,
  help,
  parseScript,
  safeScript,
  safeText,
  type CommandName,
  type ParsedCommand,
} from './commands'

const examples = [
  'help',
  'logs.query {"http_status":400,"limit":5}',
  'catalog.preview {"channels":[1]}',
  'catalog.confirm {"preview":"<preview>","channels":[1]}',
  'prices.preview {"provider":"openai","models":["<model>"],"request_rate":0}',
  'protocols.preview {"from":"openai_chat","to":"openai_responses"}',
  'changes.confirm {"preview":"<preview>","items":["<id>"]}',
]
type Entry = { line: number; name: string; status: string; result?: unknown }
function TerminalWorkspace({ scope }: { scope: NativeRoutingScope }) {
  const { t } = useTranslation()
  const [script, setScript] = useState('help')
  const [history, setHistory] = useState<string[]>([])
  const [output, setOutput] = useState<Entry[]>([])
  const [busy, setBusy] = useState(false)
  const [feedback, setFeedback] = useState('')
  const state = useRef<CatalogContext>({
    scope,
    previews: new Map(),
    catalogs: new Map(),
  })
  const active = useRef<AbortController | null>(null)
  useEffect(() => {
    const context = state.current
    const registration = new AbortController()
    const register = document.modelContext
    if (register)
      for (const [name, inputSchema] of Object.entries(help)) {
        const command = name as CommandName
        void Promise.resolve(
          register.registerTool(
            {
              name: 'lmm_terminal_' + name.replace('.', '_'),
              title: name,
              description:
                'Run this registered management terminal command using the current administrator session. A confirmation requires a previously reviewed preview ID and explicit selected items.',
              inputSchema,
              annotations: {
                readOnlyHint: !name.endsWith('.confirm'),
                consequentialHint: name.endsWith('.confirm'),
                untrustedContentHint: true,
              },
              execute: async (input, { signal }) => {
                if (active.current) throw new Error('terminal_busy')
                const controller = new AbortController()
                active.current = controller
                setBusy(true)
                try {
                  const [row] = parseScript(name + ' ' + JSON.stringify(input))
                  const result = await executeCommand(row, {
                    ...context,
                    signal: AbortSignal.any([
                      signal,
                      registration.signal,
                      controller.signal,
                    ]),
                  })
                  const sanitized = JSON.parse(safeText(JSON.stringify(result)))
                  setOutput([
                    {
                      line: 1,
                      name: command,
                      status: 'done',
                      result: sanitized,
                    },
                  ])
                  setHistory((h) => [safeScript([row]), ...h].slice(0, 30))
                  return sanitized
                } catch (e) {
                  const reason = safeText(terminalError(e))
                  setOutput([
                    {
                      line: 1,
                      name: command,
                      status: 'stopped',
                      result: reason,
                    },
                  ])
                  throw new Error(reason)
                } finally {
                  active.current = null
                  setBusy(false)
                }
              },
            },
            { signal: registration.signal }
          )
        ).catch(() => {})
      }
    return () => {
      registration.abort()
      active.current?.abort()
      context.previews.clear()
      context.catalogs.clear()
    }
  }, [])
  async function run() {
    if (active.current) return
    let commands: ParsedCommand[]
    try {
      commands = parseScript(script)
    } catch (e) {
      setOutput([
        {
          line: 0,
          name: 'script',
          status: 'error',
          result: safeText(terminalError(e)),
        },
      ])
      return
    }
    const controller = new AbortController()
    active.current = controller
    setBusy(true)
    setOutput([])
    setFeedback('')
    const saved = safeScript(commands)
    setHistory((h) => [saved, ...h.filter((x) => x !== saved)].slice(0, 30))
    try {
      for (const command of commands) {
        if (controller.signal.aborted) break
        const entry = {
          line: command.line,
          name: command.name,
          status: 'running',
        }
        setOutput((prev) => [...prev, entry])
        try {
          const result = await executeCommand(command, {
            ...state.current,
            signal: controller.signal,
          })
          const outcome =
            result && typeof result === 'object' && 'status' in result
              ? result.status
              : null
          setOutput((prev) => [
            ...prev.slice(0, -1),
            {
              ...entry,
              status: typeof outcome === 'string' ? outcome : 'done',
              result: JSON.parse(safeText(JSON.stringify(result))),
            },
          ])
          if (outcome === 'stopped' || outcome === 'conflict') break
        } catch (e) {
          setOutput((prev) => [
            ...prev.slice(0, -1),
            {
              ...entry,
              status: 'stopped',
              result:
                controller.signal.aborted && command.name.endsWith('.confirm')
                  ? 'confirmation_interrupted_check_active_version'
                  : safeText(terminalError(e)),
            },
          ])
          break
        }
      }
    } finally {
      active.current = null
      setBusy(false)
    }
  }
  async function copy(value: () => string) {
    try {
      await navigator.clipboard.writeText(value())
      setFeedback(t('Copied'))
    } catch (e) {
      setFeedback(safeText(terminalError(e)))
    }
  }
  return (
    <div className='mx-auto grid w-full max-w-4xl gap-4'>
      <label htmlFor='terminal-script' className='font-medium'>
        {t('Script')}
      </label>
      <Textarea
        id='terminal-script'
        className='min-h-48 font-mono text-sm'
        spellCheck={false}
        value={script}
        disabled={busy}
        onChange={(e) => setScript(e.target.value)}
      />
      <div className='flex flex-wrap gap-2'>
        <Button onClick={() => void run()} disabled={busy || !script.trim()}>
          {t('Run')}
        </Button>
        <Button
          variant='outline'
          disabled={!busy}
          onClick={() => active.current?.abort()}
        >
          {t('Cancel')}
        </Button>
        <Button
          variant='ghost'
          onClick={() => void copy(() => safeScript(parseScript(script)))}
        >
          {t('Copy script')}
        </Button>
      </div>
      <details>
        <summary>{t('Commands')}</summary>
        <pre className='mt-2 overflow-x-auto text-xs'>
          {examples.join('\n')}
        </pre>
      </details>
      {history.length > 0 && (
        <details>
          <summary>{t('Command history')}</summary>
          <ul className='space-y-1'>
            {history.map((item, i) => (
              <li key={i}>
                <Button
                  className='max-w-full justify-start'
                  variant='ghost'
                  disabled={busy}
                  onClick={() => setScript(item)}
                >
                  <code className='truncate'>{item}</code>
                </Button>
              </li>
            ))}
          </ul>
        </details>
      )}
      <div className='flex items-center justify-between'>
        <h2 className='font-medium'>{t('Output')}</h2>
        <Button
          variant='ghost'
          disabled={!output.length}
          onClick={() => void copy(() => JSON.stringify(output, null, 2))}
        >
          {t('Copy result')}
        </Button>
      </div>
      <div role='log' aria-live='polite' className='min-h-24 space-y-3'>
        {output.map((entry, i) => (
          <div key={i} className='border-s-2 ps-3'>
            <code>
              {entry.line} {entry.name} · {entry.status}
            </code>
            {entry.result !== undefined && (
              <pre className='max-h-96 overflow-auto text-xs break-words whitespace-pre-wrap'>
                {typeof entry.result === 'string'
                  ? entry.result
                  : JSON.stringify(entry.result, null, 2)}
              </pre>
            )}
          </div>
        ))}
      </div>
      {feedback && <p role='status'>{feedback}</p>}
    </div>
  )
}
export function ManagementTerminal() {
  const { t } = useTranslation()
  const scope = readNativeRoutingScope(useAuthStore((s) => s.auth))
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>
        {t('Management terminal')}
      </SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {scope ? (
          <TerminalWorkspace
            key={scope.userId + ':' + scope.sessionId + ':' + scope.role}
            scope={scope}
          />
        ) : (
          <p>{t('A current administrator session is required.')}</p>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
