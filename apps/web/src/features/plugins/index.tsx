/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQueryClient } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { Suspense, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ZodError } from 'zod'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import {
  isCurrentNativeUsersScope,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { SettingsPageProvider } from '@/features/system-settings/components/settings-page-context'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  changePlugin,
  reloadPlugins,
  type PluginAction,
  type PluginInfo,
} from './api'
import {
  pluginEditor,
  pluginHostFields,
  pluginNavigation,
  renderPluginEditor,
} from './configuration'
import {
  canChangePlugin,
  pluginDependencies,
  pluginDependents,
} from './lifecycle'
import { pluginPresentation } from './presentation'
import { usePlugins } from './use-plugins'

type Selection =
  | { action: 'reload' }
  | { action: PluginAction; plugin: PluginInfo }

// @context-fold begin plugin-inventory
function PluginInventory({
  scope,
  configure,
  section,
}: {
  scope: NativeUsersScope
  configure?: string
  section?: string
}) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const runtime = useRuntimeCapabilities()
  const localQuery = usePlugins(runtime ? undefined : scope)
  const query = runtime?.plugins ?? localQuery
  const phase = useCapabilityFreshness(query)
  const [actions, setActions] = useState<HTMLDivElement | null>(null)
  const [search, setSearch] = useState('')
  const [filter, setFilter] = useState('all')
  const [selection, setSelection] = useState<Selection | null>(null)
  const [pending, setPending] = useState(false)
  const [notice, setNotice] = useState<{
    success: boolean
    message: string
    detail?: string
  } | null>(null)
  const request = useRef<AbortController | null>(null)
  useEffect(() => () => request.current?.abort(), [])
  const status = isAxiosError(query.error)
    ? query.error.response?.status
    : undefined
  const errorMessage =
    query.error instanceof ZodError
      ? t('The plugin response does not match this release.')
      : status === 401
        ? t('Your session has expired. Sign in again.')
        : status === 403
          ? t('You do not have permission to read the plugin registry.')
          : status === 503
            ? t('The extension service is unavailable (503).')
            : (getServerErrorMessage(query.error) ??
              t(
                'The service is temporarily unavailable. Please try again later.'
              ))

  const registry = query.data ?? []
  const writable = phase === 'fresh' && !pending && !query.isFetching
  const selectedPlugin =
    selection && selection.action !== 'reload'
      ? registry.find((plugin) => plugin.name === selection.plugin.name)
      : undefined
  const selectionCurrent =
    !selection ||
    selection.action === 'reload' ||
    (selectedPlugin !== undefined &&
      selectedPlugin.version === selection.plugin.version &&
      selectedPlugin.api_version === selection.plugin.api_version &&
      selectedPlugin.registration_sha256 ===
        selection.plugin.registration_sha256 &&
      canChangePlugin(selection.action, selectedPlugin, registry))

  async function applySelection() {
    if (
      !selection ||
      !writable ||
      !selectionCurrent ||
      request.current ||
      !isCurrentNativeUsersScope(scope)
    ) {
      return
    }
    const controller = new AbortController()
    request.current = controller
    setPending(true)
    setNotice(null)
    try {
      if (selection.action === 'reload') {
        await reloadPlugins(scope, controller.signal)
      } else {
        await changePlugin(
          scope,
          selection.plugin,
          selection.action,
          controller.signal
        )
      }
      if (isCurrentNativeUsersScope(scope) && !controller.signal.aborted) {
        setNotice({
          success: true,
          message: t('The plugin change was applied.'),
        })
      }
    } catch (error) {
      if (isCurrentNativeUsersScope(scope) && !controller.signal.aborted) {
        setNotice({
          success: false,
          message: t(
            'The change could not be confirmed. Review the latest status before trying again.'
          ),
          detail:
            error instanceof ZodError
              ? t('The plugin response does not match this release.')
              : (getServerErrorMessage(error) ?? undefined),
        })
      }
    } finally {
      if (isCurrentNativeUsersScope(scope) && !controller.signal.aborted) {
        setSelection(null)
        // Read back once even when the write response was lost. Never replay a write.
        await query.refetch()
        if (isCurrentNativeUsersScope(scope) && !controller.signal.aborted) {
          for (const queryKey of [
            ['status'],
            ['system-options'],
            ['channel-ops'],
          ]) {
            void client.invalidateQueries({ queryKey })
          }
          setPending(false)
        }
      }
      request.current = null
    }
  }

  if (query.isPending) return <p>{t('Loading...')}</p>
  if (!query.data || (configure && query.isError)) {
    return (
      <ErrorState
        description={errorMessage}
        onRetry={() => void query.refetch()}
      />
    )
  }

  if (configure) {
    const plugin = registry.find((entry) => entry.name === configure)
    const current = phase === 'fresh'
    const enabled = plugin?.available && plugin.installed && plugin.enabled
    // The check-in policy is a host administration route. Market review is
    // shared by Store and Tool Market; either enabled owner serves it.
    const sharedReview =
      configure === 'toolmarket' &&
      !section &&
      registry.some(
        (entry) =>
          entry.name === 'store' &&
          entry.available &&
          entry.installed &&
          entry.enabled
      )
    const checkinPolicy =
      configure === 'checkins' && plugin?.available && plugin.installed
    const routeAvailable = current && (enabled || checkinPolicy || sharedReview)
    const editor = routeAvailable && renderPluginEditor(configure, section)
    let state: string | undefined
    if (!current) {
      state = t('Plugin status is not current. Refresh before relying on it.')
    } else if (!plugin) {
      if (!sharedReview) state = t('Page not found')
    } else if (!plugin.available) {
      state = t('This module has not been configured on the server.')
    } else if (!plugin.installed) {
      state = t('Not installed')
    } else if (!plugin.enabled) {
      state = t('Disabled')
    }
    return (
      <div className='space-y-6'>
        <div className='flex flex-wrap items-center justify-between gap-4'>
          <Button variant='ghost' render={<a href='/extensions/plugins' />}>
            {t('Back')}
          </Button>
          <div ref={setActions} />
        </div>
        {state && (
          <p role='status' className='text-muted-foreground text-sm'>
            {state}
          </p>
        )}
        {current && sharedReview && !enabled && (
          <p className='text-muted-foreground text-sm'>
            {t('Store')} · {t('Enabled')}
          </p>
        )}
        {routeAvailable &&
          pluginEditor(configure) &&
          pluginNavigation[configure] && (
            <nav aria-label={t('Settings')} className='flex flex-wrap gap-2'>
              {pluginNavigation[configure].map((item) => (
                <Button
                  key={item.label}
                  variant={section === item.id ? 'secondary' : 'ghost'}
                  render={
                    <a
                      href={`/extensions/plugins?configure=${encodeURIComponent(configure)}${item.id ? `&section=${item.id}` : ''}`}
                      aria-current={section === item.id ? 'page' : undefined}
                    />
                  }
                >
                  {t(item.label)}
                </Button>
              ))}
            </nav>
          )}
        {editor ? (
          <SettingsPageProvider
            actionsContainer={actions}
            suppressSectionHeader={false}
          >
            <Suspense
              fallback={<p role='status'>{t('Loading settings...')}</p>}
            >
              {editor}
            </Suspense>
          </SettingsPageProvider>
        ) : !current ? (
          <Button
            size='sm'
            variant='outline'
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          >
            {t('Refresh')}
          </Button>
        ) : routeAvailable ? (
          <p role='status'>{t('No settings found')}</p>
        ) : null}
      </div>
    )
  }

  const visible = registry.filter(
    (plugin) =>
      [plugin.name, ...Object.values(pluginPresentation(plugin.name, t))]
        .join(' ')
        .toLocaleLowerCase()
        .includes(search.trim().toLocaleLowerCase()) &&
      (filter === 'all' ||
        (filter === 'enabled'
          ? plugin.enabled
          : filter === 'disabled'
            ? plugin.installed && !plugin.enabled
            : filter === 'unavailable'
              ? !plugin.available
              : !plugin.installed))
  )
  const actionLabel = (action: PluginAction) => {
    switch (action) {
      case 'install':
        return t('Install')
      case 'enable':
        return t('Enable')
      case 'disable':
        return t('Disable')
      case 'uninstall':
        return t('Uninstall')
    }
  }
  const title =
    selection?.action === 'reload'
      ? t('Reload routes')
      : selection?.action === 'install'
        ? t('Install {{name}}?', { name: selection.plugin.name })
        : selection?.action === 'enable'
          ? t('Enable {{name}}?', { name: selection.plugin.name })
          : selection?.action === 'disable'
            ? t('Disable {{name}}?', { name: selection.plugin.name })
            : selection?.action === 'uninstall'
              ? t('Uninstall {{name}}?', { name: selection.plugin.name })
              : ''
  const description =
    selection?.action === 'reload'
      ? t(
          'Reload the enabled modules without changing their installation state.'
        )
      : selection?.action === 'install'
        ? t(
            'Install this registered module. Enable it separately when you are ready.'
          )
        : selection?.action === 'enable'
          ? t('Enable this module and its available routes.')
          : selection?.action === 'disable'
            ? t('Disable this module and remove its routes from navigation.')
            : t(
                'Uninstall this disabled module. Existing records are retained.'
              )

  return (
    <div className='@container/plugins min-w-0 space-y-6'>
      <p className='text-muted-foreground max-w-2xl text-sm leading-relaxed'>
        {t('Manage the modules available in this instance.')}
      </p>
      <dl className='bg-muted/40 grid grid-cols-3 gap-4 rounded-xl px-5 py-4'>
        {[
          [
            t('Enabled modules'),
            registry.filter((plugin) => plugin.enabled).length,
          ],
          [
            t('Installed'),
            registry.filter((plugin) => plugin.installed).length,
          ],
          [
            t('Available'),
            registry.filter((plugin) => plugin.available).length,
          ],
        ].map(([label, value]) => (
          <div key={label} className='space-y-1'>
            <dt className='text-muted-foreground text-xs'>{label}</dt>
            <dd className='text-2xl font-semibold tracking-tight tabular-nums'>
              {value}
            </dd>
          </div>
        ))}
      </dl>
      {phase !== 'fresh' && (
        <Alert role='status'>
          <AlertDescription>
            <p>
              {t('Plugin status is not current. Refresh before relying on it.')}
            </p>
            {query.isError && <p>{errorMessage}</p>}
          </AlertDescription>
        </Alert>
      )}
      {notice && (
        <Alert
          role={notice.success ? 'status' : 'alert'}
          variant={notice.success ? 'default' : 'destructive'}
        >
          <AlertDescription>
            <p>{notice.message}</p>
            {notice.detail && <p>{notice.detail}</p>}
          </AlertDescription>
        </Alert>
      )}
      <div className='flex min-w-0 flex-wrap items-center gap-3'>
        <Input
          className='min-h-11 w-full min-w-0 @min-[36rem]/plugins:w-72'
          type='search'
          aria-label={t('Search plugins')}
          placeholder={t('Search plugins')}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <select
          aria-label={t('Status')}
          className='bg-muted text-foreground min-h-11 min-w-0 rounded-md px-3 text-sm'
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        >
          <option value='all'>{t('All')}</option>
          <option value='enabled'>{t('Enabled')}</option>
          <option value='disabled'>{t('Disabled')}</option>
          <option value='uninstalled'>{t('Not installed')}</option>
          <option value='unavailable'>{t('Unavailable')}</option>
        </select>
        <div className='flex flex-wrap gap-2 @min-[36rem]/plugins:ms-auto'>
          <Button
            variant='ghost'
            className='min-h-11'
            onClick={() => void query.refetch()}
            disabled={query.isFetching || pending}
          >
            {query.isFetching ? t('Refreshing...') : t('Refresh')}
          </Button>
          <Button
            variant='outline'
            className='min-h-11'
            disabled={!writable}
            onClick={() => setSelection({ action: 'reload' })}
          >
            {t('Reload routes')}
          </Button>
        </div>
      </div>
      <p
        className='text-muted-foreground text-xs tabular-nums'
        aria-live='polite'
      >
        {t('Showing {{visible}} of {{total}} modules', {
          visible: visible.length,
          total: registry.length,
        })}
      </p>
      <section
        aria-label={t('Plugin management')}
        className='grid min-w-0 items-start gap-4 @min-[42rem]/plugins:grid-cols-2 @min-[68rem]/plugins:grid-cols-3'
      >
        {visible.length === 0 && (
          <div className='col-span-full space-y-3 py-10 text-center'>
            <p className='text-muted-foreground text-sm'>
              {t('No results found.')}
            </p>
            <Button
              variant='ghost'
              onClick={() => {
                setSearch('')
                setFilter('all')
              }}
            >
              {t('Clear filters')}
            </Button>
          </div>
        )}
        {visible.map((plugin) => {
          const action: PluginAction = !plugin.installed
            ? 'install'
            : plugin.enabled
              ? 'disable'
              : 'enable'
          const dependencies = pluginDependencies(plugin, registry)
          const dependents = pluginDependents(plugin, registry)
          const presentation = pluginPresentation(plugin.name, t)
          return (
            <article
              key={plugin.name}
              data-plugin={plugin.name}
              className='bg-card grid min-w-0 gap-4 rounded-xl p-5'
            >
              <div className='min-w-0 space-y-3'>
                <div className='flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-2'>
                  <h3 className='text-base font-semibold [overflow-wrap:anywhere]'>
                    {presentation.title}
                  </h3>
                  <Badge
                    variant='secondary'
                    className={
                      plugin.enabled ? 'text-success' : 'text-muted-foreground'
                    }
                  >
                    {!plugin.available
                      ? t('Setup required')
                      : plugin.enabled
                        ? t('Enabled')
                        : plugin.installed
                          ? t('Disabled')
                          : t('Not installed')}
                  </Badge>
                </div>
                <p className='text-muted-foreground flex min-w-0 flex-wrap items-center gap-x-2 text-xs'>
                  <code className='[overflow-wrap:anywhere]'>
                    {plugin.name}
                  </code>
                  <span aria-hidden='true'>·</span>
                  <span className='tabular-nums'>{plugin.version}</span>
                </p>
                {presentation.description && (
                  <p className='text-muted-foreground text-sm leading-relaxed'>
                    {presentation.description}
                  </p>
                )}
                {!plugin.available && (
                  <p className='text-warning text-xs leading-relaxed'>
                    {t('This module has not been configured on the server.')}
                  </p>
                )}
                {plugin.installed &&
                  !plugin.enabled &&
                  dependencies.length > 0 && (
                    <p className='text-warning text-xs [overflow-wrap:anywhere]'>
                      {t('Required modules are not enabled: {{names}}.', {
                        names: dependencies.join(', '),
                      })}
                    </p>
                  )}
                {plugin.enabled && dependents.length > 0 && (
                  <p className='text-muted-foreground text-xs [overflow-wrap:anywhere]'>
                    {t('Disable dependent modules first: {{names}}.', {
                      names: dependents.join(', '),
                    })}
                  </p>
                )}
                <details className='text-xs'>
                  <summary className='text-muted-foreground min-h-11 w-fit cursor-pointer content-center focus-visible:outline-2 focus-visible:outline-offset-2'>
                    {t('Plugin details')}
                  </summary>
                  <dl className='[&_dt]:text-muted-foreground mt-2 grid min-w-0 grid-cols-2 gap-4 [&_dd]:mt-1 [&_dd]:[overflow-wrap:anywhere]'>
                    <div>
                      <dt>{t('API version')}</dt>
                      <dd>{plugin.api_version}</dd>
                    </div>
                    <div>
                      <dt>{t('Installation status')}</dt>
                      <dd>
                        {plugin.installed ? t('Installed') : t('Not installed')}
                      </dd>
                    </div>
                    <div>
                      <dt>{t('Route configuration')}</dt>
                      <dd>{plugin.enabled ? t('Enabled') : t('Disabled')}</dd>
                    </div>
                    <div>
                      <dt>{t('Requires')}</dt>
                      <dd>{plugin.requires?.join(', ') || t('None')}</dd>
                    </div>
                    <div className='col-span-2'>
                      <dt>{t('Registration fingerprint')}</dt>
                      <dd>
                        <code>{plugin.registration_sha256}</code>
                      </dd>
                    </div>
                  </dl>
                </details>
                {!plugin.available && (
                  <details className='bg-muted/50 rounded-lg px-3 py-2 text-xs'>
                    <summary className='min-h-9 w-fit cursor-pointer content-center font-medium focus-visible:outline-2 focus-visible:outline-offset-2'>
                      {t('Host setup')}
                    </summary>
                    <div className='text-muted-foreground space-y-3 pb-2 leading-relaxed'>
                      <p>
                        {t(
                          'Select the module and configure its credentials and storage in the server configuration. Restart the extension service, then refresh this page.'
                        )}
                      </p>
                      <p>
                        <code>/etc/lmm.conf.d/extensions.toml</code>
                      </p>
                      <ul className='list-inside list-disc space-y-1'>
                        {[
                          'host.modules',
                          'host.web_compat',
                          'core.socket',
                          'core.identity_token_file',
                          ...pluginHostFields(plugin.name),
                        ].map((field) => (
                          <li key={field}>
                            <code className='[overflow-wrap:anywhere]'>
                              {field}
                            </code>
                          </li>
                        ))}
                      </ul>
                    </div>
                  </details>
                )}
              </div>
              <div className='flex flex-wrap items-center gap-2'>
                {plugin.available && (
                  <Button
                    variant={action === 'disable' ? 'outline' : 'default'}
                    className='min-h-11'
                    disabled={
                      !writable || !canChangePlugin(action, plugin, registry)
                    }
                    onClick={() => setSelection({ action, plugin })}
                  >
                    {actionLabel(action)}
                  </Button>
                )}
                {(plugin.name === 'ssmcpd' || pluginEditor(plugin.name)) &&
                  plugin.available && (
                    <Button
                      variant={plugin.available ? 'ghost' : 'outline'}
                      className='min-h-11'
                      render={
                        <a
                          href={
                            plugin.name === 'ssmcpd'
                              ? '/extensions/ssmcpd'
                              : `/extensions/plugins?configure=${encodeURIComponent(plugin.name)}`
                          }
                        />
                      }
                    >
                      {t('Settings')}
                    </Button>
                  )}
                {plugin.installed && !plugin.enabled && (
                  <Button
                    variant='ghost'
                    className='text-muted-foreground min-h-11'
                    disabled={!writable}
                    onClick={() =>
                      setSelection({ action: 'uninstall', plugin })
                    }
                  >
                    {t('Uninstall')}
                  </Button>
                )}
              </div>
            </article>
          )
        })}
      </section>
      <div className='text-muted-foreground space-y-2 text-xs leading-relaxed'>
        <p className='tabular-nums'>
          {t('Last successful read: {{time}}', {
            time: new Date(query.dataUpdatedAt).toLocaleString(),
          })}
        </p>
        <p>
          {t('Registry entries are declarations, not service health checks.')}
        </p>
      </div>
      <ConfirmDialog
        open={selection !== null}
        onOpenChange={(open) => {
          if (!open && !pending) setSelection(null)
        }}
        title={title}
        desc={description}
        confirmText={pending ? t('Loading...') : t('Confirm')}
        handleConfirm={() => void applySelection()}
        isLoading={pending}
        disabled={!writable || !selectionCurrent}
        destructive={
          selection?.action === 'disable' || selection?.action === 'uninstall'
        }
      >
        {!selectionCurrent && (
          <p className='text-destructive text-sm'>
            {t(
              'The registry changed. Close this dialog and review the latest state.'
            )}
          </p>
        )}
      </ConfirmDialog>
    </div>
  )
}
// @context-fold end plugin-inventory

export function PluginsPage({
  configure,
  section,
}: { configure?: string; section?: string } = {}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>
        {configure && pluginEditor(configure)
          ? pluginPresentation(configure, t).title
          : t('Plugin management')}
      </SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {scope ? (
          <PluginInventory
            key={[scope.userId, scope.sessionId, scope.role].join(':')}
            scope={scope}
            configure={configure}
            section={section}
          />
        ) : (
          <Alert role='alert'>
            <AlertDescription>
              {t('A current administrator session is required.')}
            </AlertDescription>
          </Alert>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
