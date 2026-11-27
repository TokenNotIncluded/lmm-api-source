/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { PluginAction, PluginInfo } from './api'

export function pluginDependencies(plugin: PluginInfo, registry: PluginInfo[]) {
  return (plugin.requires ?? []).filter(
    (name) =>
      !registry.some(
        (entry) =>
          entry.name === name &&
          entry.available &&
          entry.installed &&
          entry.enabled
      )
  )
}

export function pluginDependents(plugin: PluginInfo, registry: PluginInfo[]) {
  return registry
    .filter((entry) => entry.enabled && entry.requires?.includes(plugin.name))
    .map((entry) => entry.name)
}

export function canChangePlugin(
  action: PluginAction,
  plugin: PluginInfo,
  registry: PluginInfo[]
) {
  if (plugin.name === 'payments') return false
  switch (action) {
    case 'install':
      return plugin.available && !plugin.installed
    case 'enable':
      return (
        plugin.available &&
        plugin.installed &&
        !plugin.enabled &&
        pluginDependencies(plugin, registry).length === 0
      )
    case 'disable':
      return plugin.enabled && pluginDependents(plugin, registry).length === 0
    case 'uninstall':
      return plugin.installed && !plugin.enabled
  }
}
