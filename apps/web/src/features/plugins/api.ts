/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

const pluginSchema = z.object({
  name: z.string(),
  version: z.string(),
  api_version: z.number().int(),
  available: z.boolean(),
  installed: z.boolean(),
  enabled: z.boolean(),
  registration_sha256: z.string(),
  requires: z.array(z.string()).optional(),
  navigation_paths: z.array(z.string()).optional(),
})

const pluginListSchema = z.object({
  success: z.literal(true),
  data: z.array(pluginSchema),
})

export type PluginInfo = z.infer<typeof pluginSchema>
export type PluginAction = 'install' | 'enable' | 'disable' | 'uninstall'

export async function getPlugins(scope: NativeUsersScope, signal: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  const response = await api.get<unknown>(
    '/api/extensions/plugins',
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  return pluginListSchema.parse(response.data).data
}

export async function changePlugin(
  scope: NativeUsersScope,
  plugin: PluginInfo,
  action: PluginAction,
  signal?: AbortSignal
) {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  const options = nativeUsersOptions(scope, signal)
  const path = `/api/extensions/plugins/${encodeURIComponent(plugin.name)}`
  const response =
    action === 'install'
      ? await api.post<unknown>(
          '/api/extensions/plugins/install',
          {
            name: plugin.name,
            version: plugin.version,
            api_version: plugin.api_version,
            registration_sha256: plugin.registration_sha256,
          },
          options
        )
      : action === 'uninstall'
        ? await api.delete<unknown>(path, options)
        : await api.post<unknown>(`${path}/${action}`, undefined, options)
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  return z
    .object({
      success: z.literal(true),
      data: pluginSchema.refine(
        (result) =>
          result.name === plugin.name &&
          result.version === plugin.version &&
          result.api_version === plugin.api_version &&
          result.registration_sha256 === plugin.registration_sha256 &&
          (action === 'install'
            ? result.installed
            : action === 'enable'
              ? result.installed && result.enabled
              : action === 'disable'
                ? !result.enabled
                : !result.installed && !result.enabled)
      ),
    })
    .parse(response.data).data
}

export async function reloadPlugins(
  scope: NativeUsersScope,
  signal?: AbortSignal
) {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  const response = await api.post<unknown>(
    '/api/extensions/plugins/reload',
    undefined,
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  z.object({ success: z.literal(true), data: z.null() }).parse(response.data)
}
