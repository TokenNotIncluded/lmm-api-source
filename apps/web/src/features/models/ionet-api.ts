/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

export const ioNetSettingsSchema = z.object({
  revision: z.number().int().nonnegative(),
  provider: z.literal('io.net'),
  enabled: z.boolean(),
  configured: z.boolean(),
  can_connect: z.boolean(),
})
export type IoNetSettings = z.infer<typeof ioNetSettingsSchema>
const receiptSchema = z.object({
  operation_key: z.string(),
  kind: z.string(),
  state: z.enum(['unknown', 'succeeded', 'rejected', 'cancelled']),
  deployment_id: z.string().optional(),
  config_revision: z.number().int(),
  result: z.record(z.string(), z.unknown()).optional().nullable(),
})
export type IoNetReceipt = z.infer<typeof receiptSchema>
const pendingSchema = z.array(
  z.object({
    key: z.string(),
    path: z.string(),
    method: z.enum(['post', 'put', 'delete']),
  })
)
export type PendingIoNetOperation = z.infer<typeof pendingSchema>[number]

function ownerStorageKey() {
  const id = useAuthStore.getState().auth.user?.id
  if (!id) throw new Error('Sign in to continue')
  return `lmm.ionet.operations.v1:${id}`
}
export function pendingIoNetOperations(): PendingIoNetOperation[] {
  try {
    return pendingSchema.parse(
      JSON.parse(localStorage.getItem(ownerStorageKey()) || '[]')
    )
  } catch {
    return []
  }
}
function storePending(rows: PendingIoNetOperation[]) {
  // Keys and resource paths only: never persist form/API/registry/env secrets.
  localStorage.setItem(ownerStorageKey(), JSON.stringify(rows))
  window.dispatchEvent(new Event('lmm-ionet-operations'))
}
function finish(key: string) {
  storePending(pendingIoNetOperations().filter((p) => p.key !== key))
}

export async function readIoNetSettings(
  signal?: AbortSignal
): Promise<IoNetSettings> {
  const owner = ownerStorageKey()
  const response = await api.get<{ success: boolean; data: unknown }>(
    '/api/deployments/settings',
    { signal, disableDuplicate: true }
  )
  if (ownerStorageKey() !== owner || !response.data.success)
    throw new Error('Failed to load settings')
  return ioNetSettingsSchema.parse(response.data.data)
}
export async function saveIoNetSettings(settings: {
  revision: number
  enabled: boolean
  api_key: string
  clear_api_key: boolean
}): Promise<IoNetSettings> {
  const owner = ownerStorageKey()
  try {
    const response = await api.put<{ success: boolean; data: unknown }>(
      '/api/deployments/settings',
      settings
    )
    if (ownerStorageKey() !== owner || !response.data.success)
      throw new Error('Failed to save')
    return ioNetSettingsSchema.parse(response.data.data)
  } catch {
    throw new Error('Failed to save')
  }
}

// An existing pending operation must be explicitly resumed, not submitted from
// a new draft. Axios retries/duplicate suppression must not create a new key.
export async function ioNetMutation(
  method: 'post' | 'put' | 'delete',
  path: string,
  body?: unknown
) {
  const pending = pendingIoNetOperations()
  if (pending.some((p) => p.path === path && p.method === method))
    throw new Error(
      'The result is unknown. Resume checks the original operation without sending it again.'
    )
  const owner = ownerStorageKey()
  const key = crypto.randomUUID().replaceAll('-', '')
  const row: PendingIoNetOperation = { key, method, path }
  storePending([...pending, row])
  try {
    const response = await api.request<{ success: boolean; data: unknown }>({
      method,
      url: path,
      data: body,
      headers: { 'Idempotency-Key': key },
      disableDuplicate: true,
      validateStatus: (status) =>
        status === 200 || status === 202 || status === 422,
    })
    // Account switches cannot clear or display another owner's receipt.
    if (ownerStorageKey() !== owner) throw new Error('Sign in to continue')
    const receipt = receiptSchema.parse(response.data.data)
    if (receipt.operation_key !== key) throw new Error('Request failed')
    if (receipt.state === 'succeeded') {
      finish(key)
      return {
        success: true,
        data: receipt.result ?? { deployment_id: receipt.deployment_id },
      }
    }
    if (receipt.state === 'rejected') finish(key)
    return {
      success: false,
      message:
        receipt.state === 'unknown'
          ? 'The result is unknown. Resume checks the original operation without sending it again.'
          : 'Request failed',
    }
  } catch {
    // Keep the handle after transport loss, auth loss or invalid replies. The
    // authenticated cancel endpoint can fence an actually unstarted request.
    throw new Error(
      'The result is unknown. Resume checks the original operation without sending it again.'
    )
  }
}

export async function resumeIoNetOperation(
  key: string,
  cancel = false
): Promise<IoNetReceipt> {
  const owner = ownerStorageKey()
  const response = await api.post<{ data: unknown }>(
    `/api/deployments/operations/receipts/${encodeURIComponent(key)}/${cancel ? 'cancel' : 'recover'}`,
    {},
    {
      validateStatus: (status) =>
        status === 200 || status === 202 || status === 422,
    }
  )
  if (ownerStorageKey() !== owner) throw new Error('Sign in to continue')
  const receipt = receiptSchema.parse(response.data.data)
  if (receipt.operation_key !== key) throw new Error('Request failed')
  if (receipt.state !== 'unknown') finish(key)
  return receipt
}

export async function readPendingIoNetOperations(
  signal?: AbortSignal
): Promise<PendingIoNetOperation[]> {
  const owner = ownerStorageKey()
  const response = await api.get<{ success: boolean; data: unknown }>(
    '/api/deployments/operations',
    { signal, disableDuplicate: true }
  )
  if (ownerStorageKey() !== owner || !response.data.success)
    throw new Error('Request failed')
  const receipts = z.array(receiptSchema).parse(response.data.data)
  const rows: PendingIoNetOperation[] = []
  for (const op of receipts) {
    if (op.kind === 'create') {
      rows.push({
        key: op.operation_key,
        path: '/api/deployments/',
        method: 'post',
      })
      continue
    }
    if (!op.deployment_id) continue
    const path = '/api/deployments/' + op.deployment_id
    switch (op.kind) {
      case 'extend':
        rows.push({
          key: op.operation_key,
          path: path + '/extend',
          method: 'post',
        })
        break
      case 'rename':
        rows.push({
          key: op.operation_key,
          path: path + '/name',
          method: 'put',
        })
        break
      case 'update':
        rows.push({ key: op.operation_key, path, method: 'put' })
        break
      case 'delete':
        rows.push({ key: op.operation_key, path, method: 'delete' })
        break
    }
  }
  return rows
}
