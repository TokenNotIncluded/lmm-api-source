/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { ErrorState } from '@/components/error-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

const readEndpoint = '/api/option/epay/config'
const writeEndpoint = '/core/v1/payment-gateways/epay'
const siteMarker = '${PUBLIC_ORIGIN}'
const revisionSchema = z.string().regex(/^[a-f0-9]{64}$/)
const bindingSchema = z.object({
  channel_id: z.number().int().positive(),
  merchant_id: z.string().min(1),
  environment: z.enum(['test', 'live']),
  currency: z.enum(['LDC', 'CNY']),
  payment_type: z.string().min(1),
  endpoint: z.string(),
  notify_url: z.string(),
  return_url: z.string(),
  key_configured: z.boolean(),
  enabled: z.boolean(),
  editable: z.boolean(),
  credits_per_minor: z.string().regex(/^[1-9][0-9]*$/),
  max_amount_minor: z.string().regex(/^[1-9][0-9]*$/),
  min_amount_minor: z.string().regex(/^[1-9][0-9]*$/),
  adapter_configured: z.boolean(),
  adapter_matches: z.boolean(),
  wallet_enabled: z.boolean(),
  subscription_enabled: z.boolean(),
})
const configurationSchema = z.object({
  contract: z.literal('core.epay-configuration.v1'),
  revision: revisionSchema,
  bindings: z.array(bindingSchema),
})
type Configuration = z.infer<typeof configurationSchema>
type Binding = z.infer<typeof bindingSchema>
type Draft = {
  endpoint: string
  notify_url: string
  return_url: string
  secret_key: string
}

function minorAmount(value: string) {
  const minor = BigInt(value)
  return `${minor / 100n}.${(minor % 100n).toString().padStart(2, '0')}`
}

function draftOf(binding: Binding): Draft {
  return {
    endpoint: binding.endpoint,
    notify_url: binding.notify_url,
    return_url: binding.return_url,
    secret_key: '',
  }
}
function responseData<T>(schema: z.ZodType<T>, payload: unknown): T {
  const parsed = z
    .object({ success: z.literal(true), data: schema })
    .safeParse(payload)
  if (!parsed.success) throw new Error('Invalid EPay configuration response')
  return parsed.data.data
}
function problem(error: unknown) {
  const parsed = z
    .object({
      response: z.object({
        status: z.number(),
        data: z
          .object({
            code: z.string().optional(),
            error: z.object({ code: z.string().optional() }).optional(),
          })
          .optional(),
      }),
    })
    .safeParse(error)
  return parsed.success
    ? {
        status: parsed.data.response.status,
        code:
          parsed.data.response.data?.code ??
          parsed.data.response.data?.error?.code ??
          '',
      }
    : { status: 0, code: '' }
}
async function readConfiguration(
  scope: NativeUsersScope,
  signal?: AbortSignal
) {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new Error('Expired EPay configuration scope')
  }
  const response = await api.get<unknown>(
    readEndpoint,
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new Error('Expired EPay configuration scope')
  }
  return responseData(configurationSchema, response.data)
}

export function NativeEPaySettings() {
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const { t } = useTranslation()
  if (!scope || scope.role !== ROLE.SUPER_ADMIN) {
    return (
      <ErrorState
        title={t('Owner access required')}
        description={t(
          'Only the platform owner can manage payment credentials and gateway settings.'
        )}
      />
    )
  }
  return (
    <EPayWorkspace
      key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
      scope={scope}
    />
  )
}

function EPayWorkspace({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const queryKey = ['native-epay-configuration', scope.userId, scope.sessionId]
  const [denied, setDenied] = useState(false)
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => readConfiguration(scope, signal),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  if (
    denied ||
    (query.isError && [401, 403].includes(problem(query.error).status))
  ) {
    return (
      <ErrorState
        title={t('Payment access denied')}
        description={t(
          'Your session can no longer manage payments. Sign in with an owner account.'
        )}
      />
    )
  }
  if (query.isPending) {
    return (
      <p role='status' className='text-muted-foreground py-12 text-sm'>
        {t('Loading payment configuration...')}
      </p>
    )
  }
  if (query.isError || !query.data) {
    return (
      <ErrorState
        title={t('EPay settings unavailable')}
        description={t(
          'The payment service could not read the current EPay configuration. Retry after checking the service.'
        )}
        onRetry={() => {
          void query.refetch()
        }}
      />
    )
  }
  if (query.data.bindings.length === 0) {
    return (
      <p role='status' className='text-muted-foreground py-12 text-sm'>
        {t('No EPay connection is configured.')}
      </p>
    )
  }
  return (
    <EPayEditor
      scope={scope}
      configuration={query.data}
      queryKey={queryKey}
      onDenied={() => setDenied(true)}
    />
  )
}

function EPayEditor({
  scope,
  configuration,
  queryKey,
  onDenied,
}: {
  scope: NativeUsersScope
  configuration: Configuration
  queryKey: (string | number)[]
  onDenied: () => void
}) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const [selectedID, setSelectedID] = useState(
    configuration.bindings[0].channel_id
  )
  const current =
    configuration.bindings.find(
      (binding) => binding.channel_id === selectedID
    ) ?? configuration.bindings[0]
  const [draft, setDraft] = useState(() => draftOf(current))
  const [busy, setBusy] = useState<'save' | 'reload' | null>(null)
  const [message, setMessage] = useState('')
  const [saved, setSaved] = useState(false)
  const [confirmReload, setConfirmReload] = useState(false)
  const flight = useRef<AbortController | null>(null)
  useEffect(
    () => () => {
      flight.current?.abort()
    },
    []
  )
  const values = {
    endpoint: draft.endpoint.trim(),
    notify_url: draft.notify_url.trim(),
    return_url: draft.return_url.trim(),
    secret_key: draft.secret_key.trim(),
  }
  const dirty =
    values.endpoint !== current.endpoint ||
    values.notify_url !== current.notify_url ||
    values.return_url !== current.return_url ||
    values.secret_key !== ''
  const editable =
    current.editable &&
    !current.enabled &&
    current.currency === 'LDC' &&
    current.payment_type === 'epay' &&
    current.adapter_configured &&
    current.adapter_matches &&
    !current.wallet_enabled &&
    !current.subscription_enabled
  function rejectAccess(error: unknown) {
    if (![401, 403].includes(problem(error).status)) return false
    setDraft(draftOf(current))
    client.removeQueries({ queryKey, exact: true })
    onDenied()
    return true
  }
  function applyReadback(actual: Configuration) {
    const selected = actual.bindings.find(
      (binding) => binding.channel_id === selectedID
    )
    if (!selected) throw new Error('Selected EPay connection disappeared')
    client.setQueryData(queryKey, actual)
    setDraft(draftOf(selected))
    return selected
  }
  async function reload() {
    if (flight.current || !isCurrentNativeUsersScope(scope)) return
    const controller = new AbortController()
    flight.current = controller
    setBusy('reload')
    setConfirmReload(false)
    setSaved(false)
    setMessage('')
    try {
      const actual = await readConfiguration(scope, controller.signal)
      if (!controller.signal.aborted && isCurrentNativeUsersScope(scope)) {
        applyReadback(actual)
      }
    } catch (error) {
      if (
        !controller.signal.aborted &&
        isCurrentNativeUsersScope(scope) &&
        !rejectAccess(error)
      ) {
        setMessage(t('The current EPay configuration could not be reloaded.'))
      }
    } finally {
      if (flight.current === controller) flight.current = null
      if (!controller.signal.aborted && isCurrentNativeUsersScope(scope)) {
        setBusy(null)
      }
    }
  }
  async function save() {
    if (
      !editable ||
      !dirty ||
      flight.current ||
      !isCurrentNativeUsersScope(scope)
    ) {
      return
    }
    if (
      values.endpoint.replace(/\/+$/, '') !== 'https://credit.linux.do/epay'
    ) {
      setMessage(
        t(
          'Linux DO Credit uses https://credit.linux.do/epay as its gateway endpoint.'
        )
      )
      return
    }
    const validAddress = (value: string) => {
      try {
        const path = value.startsWith(siteMarker) ? value.slice(siteMarker.length) : null
        if (path !== null) {
          return (
            path.startsWith('/') &&
            !path.startsWith('//') &&
            !/[\\#\s]/.test(path) &&
            !path.includes('${') &&
            new TextEncoder().encode(value).length <= 100
          )
        }
        const url = new URL(value)
        return (
          !value.includes('${') &&
          url.protocol === 'https:' &&
          !url.username &&
          !url.password &&
          !value.includes('#') &&
          new TextEncoder().encode(value).length <= 100
        )
      } catch {
        return false
      }
    }
    if (!validAddress(values.notify_url) || !validAddress(values.return_url)) {
      setMessage(
        t(
          'Payment notification and return addresses must use HTTPS, contain no credentials or fragment, and fit within 100 bytes.'
        )
      )
      return
    }
    const controller = new AbortController()
    flight.current = controller
    const active = () =>
      !controller.signal.aborted && isCurrentNativeUsersScope(scope)
    setBusy('save')
    setSaved(false)
    setMessage('')
    try {
      const response = await api.put<unknown>(
        `${writeEndpoint}/${current.channel_id}`,
        {
          expected_revision: configuration.revision,
          ...values,
        },
        { ...nativeUsersOptions(scope, controller.signal), timeout: 30000 }
      )
      if (!active()) return
      const receipt = responseData(
        z.object({
          revision: revisionSchema,
          bindings: z.array(bindingSchema.pick({
            channel_id: true,
            notify_url: true,
            return_url: true,
          })).optional(),
        }),
        response.data
      )
      const actual = await readConfiguration(scope, controller.signal)
      if (!active()) return
      const selected = applyReadback(actual)
      const committed = receipt.bindings?.find((binding) => binding.channel_id === current.channel_id)
      const matchesAddress = (field: 'notify_url' | 'return_url') => {
        const submitted = values[field]
        if (!submitted.startsWith(siteMarker)) return selected[field] === submitted
        return selected[field] === committed?.[field] &&
          selected[field] === new URL(selected[field]).origin + submitted.slice(siteMarker.length)
      }
      if (
        actual.revision !== receipt.revision ||
        selected.endpoint !== values.endpoint ||
        !matchesAddress('notify_url') ||
        !matchesAddress('return_url')
      ) {
        setMessage(
          t(
            'The current configuration differs from the submitted settings. Review it before saving again.'
          )
        )
      } else {
        setSaved(true)
      }
    } catch (error) {
      if (!active() || rejectAccess(error)) return
      const failure = problem(error)
      if ([400, 409, 422].includes(failure.status)) {
        setMessage(
          failure.status === 409
            ? t(
                'The EPay configuration changed or is locked by payment history. Reload before editing again.'
              )
            : t(
                'The EPay settings were rejected. Check the addresses and replacement key.'
              )
        )
      } else {
        try {
          const actual = await readConfiguration(scope, controller.signal)
          if (!active()) return
          applyReadback(actual)
          setMessage(
            failure.code === 'PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN'
              ? t(
                  'The running settings were updated, but crash durability is unknown. Current settings were reloaded; check host storage before restarting the service.'
                )
              : t(
                  'The save could not be fully confirmed. Current settings have been reloaded; verify the server state before retrying.'
                )
          )
        } catch (readError) {
          if (!active() || rejectAccess(readError)) return
          setMessage(
            t(
              'The save outcome is unknown and current settings could not be read. Check the payment service before retrying.'
            )
          )
        }
      }
    } finally {
      if (flight.current === controller) flight.current = null
      if (active()) setBusy(null)
    }
  }
  return (
    <div className='max-w-3xl space-y-8'>
      {configuration.bindings.length > 1 && (
        <div className='space-y-2'>
          <Label htmlFor='epay-connection'>{t('Payment connection')}</Label>
          <select
            id='epay-connection'
            value={current.channel_id}
            disabled={busy !== null || dirty}
            className='border-input bg-background min-h-11 w-full rounded-md border px-3 text-sm'
            onChange={(event) => {
              if (dirty || flight.current) return
              const next = configuration.bindings.find(
                (binding) => binding.channel_id === Number(event.target.value)
              )
              if (!next) return
              setSelectedID(next.channel_id)
              setDraft(draftOf(next))
              setMessage('')
              setSaved(false)
            }}
          >
            {configuration.bindings.map((binding) => (
              <option key={binding.channel_id} value={binding.channel_id}>
                {t('{{currency}} · Channel #{{id}}', {
                  currency: binding.currency,
                  id: binding.channel_id,
                })}
              </option>
            ))}
          </select>
        </div>
      )}
      <section className='space-y-5' aria-labelledby='epay-heading'>
        <div className='flex flex-wrap items-start justify-between gap-3'>
          <div className='space-y-1'>
            <h2 id='epay-heading' className='text-lg font-semibold'>
              {t('Epay Gateway')}
            </h2>
            <p className='text-muted-foreground text-sm'>
              {t('Current payment connection')}
            </p>
          </div>
          <Badge variant='secondary'>
            {current.environment === 'live'
              ? t('Production')
              : t('Test environment')}
          </Badge>
        </div>
        <dl className='grid gap-x-8 gap-y-5 sm:grid-cols-2'>
          {[
            [t('Merchant ID'), current.merchant_id],
            [t('Settlement unit'), current.currency],
            [t('Payment method'), current.payment_type],
            [
              t('Minimum wallet top-up'),
              `${minorAmount(current.min_amount_minor)} ${current.currency}`,
            ],
            [
              t('Maximum wallet top-up'),
              `${minorAmount(current.max_amount_minor)} ${current.currency}`,
            ],
            [
              t('Base conversion'),
              t('{{credits}} CREDIT per 0.01 {{currency}}', {
                credits: current.credits_per_minor,
                currency: current.currency,
              }),
            ],
            [
              t('Environment'),
              current.environment === 'live'
                ? t('Production')
                : t('Test environment'),
            ],
            [
              t('Signing key'),
              current.key_configured ? t('Configured') : t('Not configured'),
            ],
          ].map(([label, value]) => (
            <div key={label} className='min-w-0 space-y-1'>
              <dt className='text-muted-foreground text-xs'>{label}</dt>
              <dd className='font-mono text-sm break-all'>{value}</dd>
            </div>
          ))}
        </dl>
        <dl className='grid gap-4 sm:grid-cols-2'>
          {[
            [
              t('Core payment policy'),
              current.enabled ? t('Enabled') : t('Disabled'),
            ],
            [
              t('Payment adapter'),
              !current.adapter_configured
                ? t('Not configured')
                : current.adapter_matches
                  ? t('Configured')
                  : t('Configuration mismatch'),
            ],
            [
              t('Wallet top-ups'),
              current.wallet_enabled ? t('Enabled') : t('Disabled'),
            ],
            [
              t('Subscription checkout'),
              current.subscription_enabled ? t('Enabled') : t('Disabled'),
            ],
          ].map(([label, value]) => (
            <div key={label} className='space-y-1'>
              <dt className='text-muted-foreground text-xs'>{label}</dt>
              <dd className='text-sm'>{value}</dd>
            </div>
          ))}
        </dl>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Only inactive Linux DO Credit connections with no payment history can be edited here. Merchant, currency, conversion and activation remain fixed.'
          )}
        </p>
        {current.currency === 'LDC' &&
          !current.enabled &&
          !current.wallet_enabled &&
          !current.subscription_enabled && (
            <p className='text-muted-foreground text-sm'>
              {t(
                'This connection remains disabled. Legacy amount discounts require a separate pricing review before activation.'
              )}
            </p>
          )}
        {(!current.adapter_configured || !current.adapter_matches) && (
          <p role='alert' className='text-sm'>
            {t(
              'The payment adapter does not match this connection. Reconcile the service configuration before editing.'
            )}
          </p>
        )}
      </section>
      <form
        className='space-y-6'
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        {(
          [
            ['endpoint', t('Epay endpoint')],
            ['notify_url', t('Payment notification URL')],
            ['return_url', t('Payment return URL')],
          ] as const
        ).map(([field, label]) => (
          <div key={field} className='space-y-2'>
            <Label htmlFor={`epay-${field}`}>{label}</Label>
            <Input
              id={`epay-${field}`}
              type={field === 'endpoint' ? 'url' : 'text'}
              inputMode='url'
              placeholder={field === 'notify_url' ? `${siteMarker}/callbacks/${current.channel_id}` : field === 'return_url' ? `${siteMarker}/console/wallet` : undefined}
              value={draft[field]}
              required
              disabled={!editable || busy !== null}
              autoComplete='off'
              maxLength={field === 'endpoint' ? 512 : 100}
              aria-describedby={`epay-${field}-help`}
              onChange={(event) => {
                setDraft((previous) => ({
                  ...previous,
                  [field]: event.target.value,
                }))
                setSaved(false)
              }}
            />
            <p
              id={`epay-${field}-help`}
              className='text-muted-foreground text-xs'
            >
              {field === 'endpoint'
                ? t(
                    'Linux DO Credit uses https://credit.linux.do/epay as its gateway endpoint.'
                  )
                : field === 'notify_url'
                  ? t(
                      'The provider sends signed payment notifications here. Use the public HTTPS callback address for this channel.'
                    )
                  : t(
                      'Customers return here after checkout. This address does not confirm payment.'
                    )}
            </p>
          </div>
        ))}
        <div className='space-y-2'>
          <Label htmlFor='epay-secret-key'>
            {t('Replace EPay signing key')}
          </Label>
          <Input
            id='epay-secret-key'
            type='password'
            value={draft.secret_key}
            disabled={!editable || busy !== null}
            autoComplete='new-password'
            aria-describedby='epay-secret-help'
            onChange={(event) => {
              setDraft((previous) => ({
                ...previous,
                secret_key: event.target.value,
              }))
              setSaved(false)
            }}
          />
          <p id='epay-secret-help' className='text-muted-foreground text-xs'>
            {t(
              'Leave empty to keep the configured key. Saved keys are never returned by this page.'
            )}
          </p>
        </div>
        {message && (
          <p role='alert' className='bg-muted/40 rounded-lg p-4 text-sm'>
            {message}
          </p>
        )}
        {saved && (
          <p role='status' className='text-sm'>
            {t(
              'EPay settings saved and verified against the running configuration.'
            )}
          </p>
        )}
        <div className='flex flex-wrap gap-3'>
          <Button type='submit' disabled={!editable || !dirty || busy !== null}>
            {busy === 'save' ? t('Saving...') : t('Save EPay settings')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={!dirty || busy !== null}
            onClick={() => {
              setDraft(draftOf(current))
              setSaved(false)
              setMessage('')
            }}
          >
            {t('Discard changes')}
          </Button>
          <Button
            type='button'
            variant='ghost'
            disabled={busy !== null}
            onClick={() => {
              if (dirty) setConfirmReload(true)
              else void reload()
            }}
          >
            {t('Reload configuration')}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={confirmReload}
        onOpenChange={setConfirmReload}
        title={t('Discard changes and reload')}
        desc={t(
          'Reloading replaces your unsaved payment settings with the current server configuration.'
        )}
        confirmText={t('Discard changes and reload')}
        isLoading={busy === 'reload'}
        handleConfirm={() => {
          void reload()
        }}
      />
    </div>
  )
}
