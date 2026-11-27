/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { nativeProtocols } from '@/features/channels/lib/native-routing-form'
import { readLogsPage } from '@/features/usage-logs/api'

import { confirmCatalog, previewCatalog, type CatalogContext } from './catalog'
import { executeTerminalOperation } from './operations'

const identifier = z
  .string()
  .min(1)
  .max(200)
  .regex(/^[^\s\x00-\x1f\x7f]+$/u)
const positive = z.number().int().positive().safe()
const list = <T extends z.ZodType>(schema: T) =>
  z
    .array(schema)
    .min(1)
    .max(100)
    .refine((v) => new Set(v).size === v.length)
const reason = z.enum([
  'catalog-refresh',
  'missing-price',
  'protocol-correction',
  'operator-review',
])
const schemas = {
  help: z.strictObject({}),
  'logs.query': z.strictObject({
    request_id: identifier.optional(),
    model: identifier.optional(),
    channel: positive.optional(),
    http_status: z.number().int().min(100).max(599).optional(),
    settlement_status: z
      .enum(['pending', 'settled', 'released', 'refunded'])
      .optional(),
    page: positive.optional(),
    limit: positive.max(100).optional(),
  }),
  'catalog.preview': z.strictObject({
    channels: list(positive),
    reason: z
      .enum(['catalog-refresh', 'operator-review'])
      .default('catalog-refresh'),
  }),
  'catalog.confirm': z.strictObject({
    preview: z.uuid(),
    channels: list(positive),
  }),
  'prices.preview': z.strictObject({
    provider: identifier,
    models: list(identifier),
    request_rate: z.literal(0).optional(),
    reason: reason.optional(),
  }),
  'protocols.preview': z.strictObject({
    from: z.enum(nativeProtocols),
    to: z.enum(nativeProtocols),
    channels: list(positive).optional(),
    reason: reason.optional(),
  }),
  'changes.confirm': z.strictObject({
    preview: z.uuid(),
    items: list(identifier),
  }),
}
export type CommandName = keyof typeof schemas
export const help = Object.fromEntries(
  Object.entries(schemas).map(([name, schema]) => [
    name,
    z.toJSONSchema(schema),
  ])
)
export type ParsedCommand = {
  line: number
  name: CommandName
  args: Record<string, unknown>
}
export function parseScript(script: string): ParsedCommand[] {
  if (script.length > 65536) throw new Error('script_too_long')
  const commands: ParsedCommand[] = []
  for (const [index, line] of script.split(/\r?\n/).entries()) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) continue
    const match = /^([a-z.]+)(?:\s+(.+))?$/.exec(trimmed)
    if (!match || !Object.hasOwn(schemas, match[1]))
      throw new Error('line ' + (index + 1) + ': unknown_command')
    const name = match[1] as CommandName
    let args: unknown
    try {
      args = JSON.parse(match[2] ?? '{}')
    } catch {
      throw new Error('line ' + (index + 1) + ' ' + name + ': invalid_json')
    }
    const result = schemas[name].safeParse(args)
    if (!result.success)
      throw new Error(
        'line ' +
          (index + 1) +
          ' ' +
          name +
          ': ' +
          result.error.issues
            .map((i) => [i.path.join('.'), i.code].join(' '))
            .join(', ')
      )
    commands.push({ line: index + 1, name, args: result.data })
  }
  if (!commands.length || commands.length > 100)
    throw new Error('command_count_must_be_1_to_100')
  return commands
}
export async function executeCommand(
  command: ParsedCommand,
  context: CatalogContext
) {
  const { name, args } = command
  context.signal?.throwIfAborted()
  switch (name) {
    case 'help':
      return help
    case 'catalog.preview': {
      const p = schemas[name].parse(args)
      return previewCatalog(p.channels, p.reason, context)
    }
    case 'catalog.confirm': {
      const p = schemas[name].parse(args)
      return confirmCatalog(p.preview, p.channels, context)
    }
    case 'logs.query': {
      const p = schemas[name].parse(args)
      const r = await readLogsPage({
        category: 'common',
        isAdmin: true,
        signal: context.signal,
        scope: context.scope,
        params: {
          request_id: p.request_id,
          model_name: p.model,
          channel: p.channel,
          http_status: p.http_status,
          settlement_status: p.settlement_status,
          p: p.page,
          page_size: p.limit,
        },
      })
      if (r.kind !== 'common') throw new Error('native_logs_unavailable')
      return {
        page: r.page,
        total: r.total,
        items: r.items.map(
          ({
            request_id,
            model_name,
            channel,
            status_code,
            request_result,
            settlement_status,
          }) => ({
            request_id,
            model_name,
            channel,
            status_code,
            request_result,
            settlement_status,
          })
        ),
      }
    }
    default:
      return executeTerminalOperation(name, args, context)
  }
}
// Only validated commands enter history. Never copy raw input, error bodies or headers.
export function safeText(text: string) {
  return text
    .replace(
      /(?:Bearer\s+|(?:sk-|sk_|gh[pousr]_|github_pat_|xox[baprs]-|AKIA|AIza|eyJ[A-Za-z0-9_-]*\.))[A-Za-z0-9._~+/=-]+/gi,
      '[redacted]'
    )
    .replace(/([?&](?:token|key|secret|password)=)[^&\s"]+/gi, '$1[redacted]')
}
export const safeScript = (cmds: ParsedCommand[]) =>
  cmds
    .map(
      (c) =>
        c.name +
        (Object.keys(c.args).length
          ? ' ' + safeText(JSON.stringify(c.args))
          : '')
    )
    .join('\n')
