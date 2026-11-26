/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQueryClient } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useCallback, useEffect, useRef, useState } from 'react'
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
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { ROLE } from '@/lib/roles'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  changePlugin,
  changePluginOrder,
  reloadPlugins,
  type PluginAction,
  type PluginInfo,
} from './api'
import {
  canOpenConfiguration,
  configurationOwner,
  pluginHostFields,
} from './configuration'
import { PluginConfiguration } from './configuration-panel'
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
type ConfigurationProps = {
  panel?: string
  onPanelChange?: (panel: string | undefined) => void
}

function PluginInventory({
  scope,
  panel,
  onPanelChange,
}: { scope: NativeUsersScope } & ConfigurationProps) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const runtime = useRuntimeCapabilities()
  const localQuery = usePlugins(runtime ? undefined : scope)
  const query = runtime?.plugins ?? localQuery
  const phase = useCapabilityFreshness(query)
  const [search, setSearch] = useState('')
  const [localPanel, setLocalPanel] = useState<string>()
  const requested = onPanelChange ? panel : localPanel
  const root = scope.role === ROLE.SUPER_ADMIN
  const activePanel = canOpenConfiguration(requested, root)
    ? requested
    : undefined
  const changePanel = onPanelChange ?? setLocalPanel
  const openingTrigger = useRef<HTMLButtonElement | null>(null)
  const returnTrigger = useRef<HTMLButtonElement | null>(null)
  const focusPanel = useCallback((element: HTMLElement | null) => {
    if (!element) return
    const trigger =
      element
        .closest('article')
        ?.querySelector<HTMLButtonElement>('[data-configure-plugin]') ?? null
    returnTrigger.current = trigger
    // Opening an accordion from its own button keeps that button in the tab order.
    // A deep link lands directly on the configuration, including on narrow screens.
    if (openingTrigger.current !== trigger || !trigger) {
      element.focus({ preventScroll: true })
      element.scrollIntoView?.({ block: 'start' })
    }
    openingTrigger.current = null
  }, [])
  useEffect(() => {
    if (!activePanel && returnTrigger.current) {
      returnTrigger.current.focus({ preventScroll: true })
      returnTrigger.current = null
    }
  }, [activePanel])
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

  async function applySelection(order?: string[] | null) {
    if (
      (order === undefined && (!selection || !selectionCurrent)) ||
      !writable ||
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
      if (order !== undefined) {
        await changePluginOrder(scope, order, controller.signal)
      } else if (selection?.action === 'reload') {
        await reloadPlugins(scope, controller.signal)
      } else if (selection) {
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
  if (!query.data) {
    return (
      <ErrorState
        description={errorMessage}
        onRetry={() => void query.refetch()}
      />
    )
  }

  // The runtime registry also carries adapters for core services. Keep the
  // full registry for dependency checks, but only manage extensions here.
  const extensions = registry.filter(
    (plugin) => plugin.name !== 'identity' && plugin.name !== 'payments'
  )
  const enabled = registry
    .filter((plugin) => plugin.enabled)
    .sort((left, right) => (left.load_order ?? 0) - (right.load_order ?? 0))
  const orderReady = enabled.every(
    (plugin, index) => plugin.load_order === index + 1
  )
  const orderedExtensions = enabled.filter((plugin) =>
    extensions.includes(plugin)
  )

  function movedOrder(position: number, offset: number) {
    const selected = orderedExtensions[position]
    const adjacent = orderedExtensions[position + offset]
    if (!selected || !adjacent) return null
    const names = enabled.map((plugin) => plugin.name)
    const from = names.indexOf(selected.name)
    const to = names.indexOf(adjacent.name)
    ;[names[from], names[to]] = [names[to], names[from]]
    const positions = new Map(names.map((name, index) => [name, index]))
    const valid = enabled.every((plugin) =>
      (plugin.requires ?? []).every(
        (dependency) =>
          (positions.get(dependency) ?? names.length) <
          (positions.get(plugin.name) ?? -1)
      )
    )
    return valid ? names : null
  }

  const visible = extensions.filter(
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
            extensions.filter((plugin) => plugin.enabled).length,
          ],
          [
            t('Installed'),
            extensions.filter((plugin) => plugin.installed).length,
          ],
          [
            t('Available'),
            extensions.filter((plugin) => plugin.available).length,
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
      {phase === 'fresh' &&
        activePanel &&
        !extensions.some(
          (plugin) => plugin.name === configurationOwner(activePanel)
        ) && <p role='status'>{t('No results found.')}</p>}
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
          total: extensions.length,
        })}
      </p>
      {orderReady && orderedExtensions.length > 0 && (
        <section aria-labelledby='plugin-load-order' className='space-y-3'>
          <div className='flex flex-wrap items-center justify-between gap-2'>
            <h2 id='plugin-load-order' className='text-base font-semibold'>
              {t('Plugin load order')}
            </h2>
            <Button
              variant='outline'
              className='min-h-11'
              disabled={!writable}
              onClick={() => void applySelection(null)}
            >
              {t('Sort by dependencies')}
            </Button>
          </div>
          <ol className='divide-border divide-y'>
            {orderedExtensions.map((plugin, position) => {
              const up = movedOrder(position, -1)
              const down = movedOrder(position, 1)
              const { title } = pluginPresentation(plugin.name, t)
              return (
                <li
                  key={plugin.name}
                  className='flex flex-wrap items-center justify-between gap-2 py-2'
                >
                  <span className='min-w-0 text-sm break-words'>
                    <span className='text-muted-foreground me-2 tabular-nums'>
                      {plugin.load_order}.
                    </span>
                    {title}
                  </span>
                  <div
                    className='flex flex-wrap gap-1'
                    role='group'
                    aria-label={title}
                  >
                    <Button
                      size='sm'
                      variant='ghost'
                      className='min-h-11'
                      disabled={!writable || !up}
                      onClick={() => up && void applySelection(up)}
                    >
                      {t('Move up')}
                    </Button>
                    <Button
                      size='sm'
                      variant='ghost'
                      className='min-h-11'
                      disabled={!writable || !down}
                      onClick={() => down && void applySelection(down)}
                    >
                      {t('Move down')}
                    </Button>
                  </div>
                </li>
              )
            })}
          </ol>
        </section>
      )}
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
          const hostFields = pluginHostFields(plugin.name)
          const configOpen = configurationOwner(activePanel) === plugin.name
          const configId = `plugin-config-${plugin.name}`
          // Only the installed check-in settings endpoint is host-owned and
          // remains available when its business module is switched off.
          const configReady =
            plugin.enabled || (plugin.name === 'checkins' && plugin.installed)
          const lifecycleControl = plugin.available && (
            <Button
              variant={action === 'disable' ? 'outline' : 'default'}
              className='min-h-11'
              disabled={!writable || !canChangePlugin(action, plugin, registry)}
              onClick={() => setSelection({ action, plugin })}
            >
              {actionLabel(action)}
            </Button>
          )
          return (
            <article
              key={plugin.name}
              data-plugin={plugin.name}
              className={`bg-card grid min-w-0 gap-4 rounded-xl p-5 ${configOpen ? 'col-span-full' : ''}`}
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
                    <div>
                      <dt>{t('Required by')}</dt>
                      <dd>
                        {(plugin.required_by ?? dependents).join(', ') ||
                          t('None')}
                      </dd>
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
                          ...(hostFields ?? []),
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
                {(!configOpen || configReady) && lifecycleControl}
                {canOpenConfiguration(plugin.name, root) && (
                  <Button
                    type='button'
                    variant={plugin.available ? 'ghost' : 'outline'}
                    className='min-h-11'
                    data-configure-plugin={plugin.name}
                    aria-expanded={configOpen}
                    aria-controls={configOpen ? configId : undefined}
                    onClick={(event) => {
                      openingTrigger.current = configOpen
                        ? null
                        : event.currentTarget
                      changePanel(configOpen ? undefined : plugin.name)
                    }}
                  >
                    {t('Configure')}
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
              {configOpen && activePanel && (
                <section
                  id={configId}
                  ref={focusPanel}
                  tabIndex={-1}
                  aria-label={t('Configuration')}
                  className='min-w-0 scroll-mt-16 border-t pt-4 focus-visible:outline-offset-2'
                >
                  {configReady ? (
                    <PluginConfiguration
                      name={activePanel}
                      writable={writable}
                      onSelect={changePanel}
                    />
                  ) : (
                    <div className='flex flex-wrap items-center gap-3'>
                      <p
                        role='status'
                        className='text-muted-foreground text-sm'
                      >
                        {!plugin.available
                          ? t('Unavailable')
                          : plugin.installed
                            ? t('Enable this module and its available routes.')
                            : t(
                                'Install this registered module. Enable it separately when you are ready.'
                              )}
                      </p>
                      {lifecycleControl}
                    </div>
                  )}
                  <Button
                    type='button'
                    variant='ghost'
                    className='mt-4 min-h-11'
                    onClick={() => changePanel(undefined)}
                  >
                    {t('Close')}
                  </Button>
                </section>
              )}
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

export function PluginsPage(props: ConfigurationProps = {}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>
        {t('Plugin management')}
      </SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {scope ? (
          <PluginInventory
            key={[scope.userId, scope.sessionId, scope.role].join(':')}
            scope={scope}
            {...props}
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
