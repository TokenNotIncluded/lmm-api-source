/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Check, Copy, RefreshCw, ShieldCheck } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { ErrorState } from '@/components/error-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

const endpoint = '/api/option/waffo-pancake'
const bindingSchema = z.object({
  channel_id: z.number().int().positive().optional(),
  merchant_id: z.string().min(1),
  store_id: z.string().min(1),
  product_id: z.string().min(1),
  return_url: z.string(),
  environment: z.enum(['test', 'live']),
  private_key_configured: z.boolean(),
  channels: z
    .array(
      z.object({
        id: z.number().int().positive(),
        currency: z.string().min(1),
        enabled: z.boolean(),
        subscription_enabled: z.boolean(),
      })
    )
    .min(1),
})
const configurationSchema = z.union([
  z.object({
    contract: z.literal('extensions.pancake-configuration.v1'),
    bindings: z
      .array(
        bindingSchema.extend({
          channel_id: z.number().int().positive(),
        })
      )
      .min(1),
  }),
  bindingSchema
    .extend({
      contract: z.literal('extensions.pancake-configuration.v1'),
    })
    .transform((binding) => ({
      contract: binding.contract,
      bindings: [binding],
    })),
])

const productSchema = z.object({
  id: z.string().min(1),
  name: z.string(),
  status: z.string(),
})
const catalogSchema = z.object({
  stores: z.array(
    z.object({
      id: z.string().min(1),
      name: z.string(),
      status: z.string(),
      prodEnabled: z.boolean(),
      onetimeProducts: z.array(productSchema),
      subscriptionProducts: z.array(productSchema),
    })
  ),
})
const pairSchema = z.object({
  store_id: z.string().min(1),
  store_name: z.string(),
  product_id: z.string().optional(),
  product_name: z.string().optional(),
  orphan_store: z.boolean().optional(),
})
const saveSchema = z.object({ store_id: z.string(), product_id: z.string() })
type Configuration = z.infer<typeof configurationSchema>
type PaymentBinding = z.infer<typeof bindingSchema>
type Catalog = z.infer<typeof catalogSchema>
type Pair = z.infer<typeof pairSchema>
type Action = 'catalog' | 'pair' | 'save'

function bindingID(binding: PaymentBinding): number {
  return binding.channel_id ?? binding.channels[0].id
}

function selectedBinding(
  configuration: Configuration,
  id: number
): PaymentBinding {
  const binding = configuration.bindings.find(
    (item) =>
      bindingID(item) === id ||
      item.channels.some((channel) => channel.id === id)
  )
  if (!binding) throw new PaymentFailure(0, 'INVALID_RESPONSE')
  return binding
}

class PaymentFailure extends Error {
  constructor(
    readonly status = 0,
    readonly code = '',
    readonly partial?: Pair
  ) {
    super('Payment request failed')
  }
}

function failure(error: unknown): PaymentFailure {
  if (error instanceof PaymentFailure) return error
  const parsed = z
    .object({
      response: z.object({ status: z.number(), data: z.unknown().optional() }),
    })
    .safeParse(error)
  if (!parsed.success) return new PaymentFailure()
  const body = z
    .object({
      code: z.string().optional(),
      data: z.unknown().optional(),
    })
    .safeParse(parsed.data.response.data)
  const partial = body.success ? pairSchema.safeParse(body.data.data) : null
  return new PaymentFailure(
    parsed.data.response.status,
    body.success ? body.data.code : undefined,
    partial?.success ? partial.data : undefined
  )
}

function responseData<T>(schema: z.ZodType<T>, payload: unknown): T {
  const parsed = z
    .object({ success: z.literal(true), data: schema })
    .safeParse(payload)
  if (!parsed.success) throw new PaymentFailure(0, 'INVALID_RESPONSE')
  return parsed.data.data
}

async function readConfiguration(
  scope: NativeUsersScope,
  signal?: AbortSignal
) {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new PaymentFailure()
  }
  const response = await api.get<unknown>(
    `${endpoint}/config`,
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new PaymentFailure()
  }
  return responseData(configurationSchema, response.data)
}

const hostExample = `[host]
modules = ["payments"]
web_compat = true

[core]
socket = "/run/lmm/core/core.sock"
identity_token_file = "/etc/lmm.conf.d/secrets/extensions/core-rpc-token"
payment_token_file = "/etc/lmm.conf.d/secrets/extensions/payment-rpc-token"

[payments]
database_name = "lmm_payments"
database_url_file = "/etc/lmm.conf.d/secrets/extensions/payment-database-url"
config_file = "/etc/lmm.conf.d/extensions-payments.toml"
callbacks_enabled = true`

const paymentExample = `version = 2

[[channels]]
id = 7
provider = "waffo_pancake"
merchant = "MER_REPLACE_WITH_YOUR_MERCHANT"
environment = "test"
currency = "USD"
enabled = false
subscription_enabled = false
private_key_file = "/etc/lmm.conf.d/secrets/extensions/pancake-private-key.pem"
store_id = "STO_REPLACE_WITH_YOUR_STORE"
product_id = "PROD_REPLACE_WITH_YOUR_PRODUCT"
return_url = "https://example.com/wallet"
checkout_region = "global"
checkout_language = "en"
tax_category = "saas"`

function ConfigurationExample({
  path,
  content,
}: {
  path: string
  content: string
}) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  return (
    <div className='space-y-3'>
      <p className='font-mono text-xs'>{path}</p>
      <pre className='bg-background overflow-x-auto rounded-lg p-4 text-xs leading-relaxed'>
        {content}
      </pre>
      <Button
        variant='outline'
        size='sm'
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(content)
            setCopied(true)
          } catch {
            setCopied(false)
          }
        }}
      >
        {copied ? <Check className='size-4' /> : <Copy className='size-4' />}
        {copied ? t('Copied') : t('Copy example')}
      </Button>
    </div>
  )
}

function HostSetup() {
  const { t } = useTranslation()
  return (
    <details className='bg-muted/30 rounded-xl p-5'>
      <summary className='cursor-pointer text-sm font-medium'>
        {t('Host setup')}
      </summary>
      <div className='mt-4 space-y-4 text-sm'>
        <p className='text-muted-foreground max-w-2xl'>
          {t(
            'Merge these payment fields into the existing host configuration. Provision the private credential files, payment database, channel configuration and matching Core payment policy, then restart the extension service.'
          )}
        </p>
        <ConfigurationExample
          path='/etc/lmm.conf.d/extensions.toml'
          content={hostExample}
        />
        <p className='text-muted-foreground'>
          {t(
            'Keep existing modules and configuration. This example contains paths only; private credentials must be provisioned on the host. Payment callbacks also require the configured public ingress.'
          )}
        </p>
        <ConfigurationExample
          path='/etc/lmm.conf.d/extensions-payments.toml'
          content={paymentExample}
        />
        <p className='text-muted-foreground'>
          {t(
            'Replace all example identifiers and match the channel ID, merchant, environment, currency, store and product with the Core payment policy before enabling the channel.'
          )}
        </p>
      </div>
    </details>
  )
}

export function NativePaymentSettings() {
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
    <PaymentWorkspace
      key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
      scope={scope}
    />
  )
}

function PaymentWorkspace({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const queryKey = [
    'native-payment-configuration',
    scope.userId,
    scope.sessionId,
  ]
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => readConfiguration(scope, signal),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  const [denied, setDenied] = useState(false)
  if (
    denied ||
    (query.isError && [401, 403].includes(failure(query.error).status))
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
      <div className='space-y-6'>
        <ErrorState
          title={t('Payment settings unavailable')}
          description={
            failure(query.error).status === 409
              ? t(
                  'The host payment configuration changed or has multiple distinct bindings. Review the host configuration and reload the service.'
                )
              : t(
                  'The payment service could not provide its configuration. Check the service and host setup, then retry.'
                )
          }
          onRetry={() => {
            void query.refetch()
          }}
        />
        <HostSetup />
      </div>
    )
  }
  return (
    <PaymentEditor
      scope={scope}
      configuration={query.data}
      queryKey={queryKey}
      onDenied={() => setDenied(true)}
    />
  )
}

function PaymentEditor({
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
  const [selectedID, setSelectedID] = useState(() =>
    bindingID(configuration.bindings[0])
  )
  const current = selectedBinding(configuration, selectedID)
  const [returnURL, setReturnURL] = useState(current.return_url)
  const [privateKey, setPrivateKey] = useState('')
  const [busy, setBusy] = useState<Action | null>(null)
  const [message, setMessage] = useState('')
  const [saved, setSaved] = useState(false)
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [pair, setPair] = useState<Pair | null>(null)
  const [confirmPair, setConfirmPair] = useState(false)
  const inFlight = useRef<AbortController | null>(null)
  useEffect(
    () => () => {
      inFlight.current?.abort()
    },
    []
  )

  const dirty =
    returnURL.trim() !== current.return_url || privateKey.trim() !== ''
  const validReturnURL = () => {
    if (!returnURL.trim()) return true
    try {
      const url = new URL(returnURL.trim())
      return (
        url.protocol === 'https:' &&
        !url.username &&
        !url.password &&
        (!url.port || url.port === '443')
      )
    } catch {
      return false
    }
  }
  const rejectAccess = (error: unknown) => {
    if (![401, 403].includes(failure(error).status)) return false
    setPrivateKey('')
    client.removeQueries({ queryKey, exact: true })
    onDenied()
    return true
  }

  async function run(action: Action) {
    if (inFlight.current || !isCurrentNativeUsersScope(scope)) return
    if (action !== 'catalog' && !validReturnURL()) {
      setMessage(
        t('Enter an HTTPS return URL without credentials, or leave it empty.')
      )
      return
    }
    const controller = new AbortController()
    inFlight.current = controller
    const active = () =>
      !controller.signal.aborted && isCurrentNativeUsersScope(scope)
    setBusy(action)
    setMessage('')
    setSaved(false)
    if (action === 'catalog') setCatalog(null)
    setConfirmPair(false)
    const values = {
      ...(current.channel_id === undefined
        ? {}
        : { channel_id: current.channel_id }),
      merchant_id: current.merchant_id,
      private_key: privateKey.trim(),
      return_url: returnURL.trim(),
      store_id: current.store_id,
      product_id: current.product_id,
    }
    let acknowledged = false
    try {
      const response = await api.post<unknown>(
        `${endpoint}/${action}`,
        values,
        {
          ...nativeUsersOptions(scope, controller.signal),
          timeout: 30000,
        }
      )
      if (!active()) return
      if (action === 'catalog') {
        setCatalog(responseData(catalogSchema, response.data))
      } else if (action === 'pair') {
        const result = responseData(pairSchema, response.data)
        if (!result.product_id || result.orphan_store) {
          throw new PaymentFailure(502, 'PAYMENT_PAIR_INCOMPLETE', result)
        }
        setPair(result)
      } else {
        const result = responseData(saveSchema, response.data)
        if (
          result.store_id !== values.store_id ||
          result.product_id !== values.product_id
        ) {
          throw new PaymentFailure(0, 'INVALID_RESPONSE')
        }
        acknowledged = true
        const refreshed = await readConfiguration(scope, controller.signal)
        if (!active()) return
        const actual = selectedBinding(refreshed, selectedID)
        client.setQueryData(queryKey, refreshed)
        setSelectedID(bindingID(actual))
        setReturnURL(actual.return_url)
        setPrivateKey('')
        if (
          actual.return_url !== values.return_url ||
          actual.merchant_id !== values.merchant_id ||
          actual.store_id !== values.store_id ||
          actual.product_id !== values.product_id
        ) {
          setMessage(
            t(
              'The current configuration differs from the submitted settings. Review it before saving again.'
            )
          )
        } else {
          setSaved(true)
        }
      }
    } catch (error) {
      if (!active() || rejectAccess(error)) return
      const problem = failure(error)
      if (
        action === 'pair' &&
        problem.code === 'PAYMENT_PAIR_INCOMPLETE' &&
        problem.partial
      ) {
        setPair(problem.partial)
        setMessage(
          t(
            'The store was created, but product setup is incomplete. Review the returned store before trying again.'
          )
        )
      } else if (problem.code === 'PAYMENT_CORE_POLICY_REQUIRED') {
        setMessage(
          t(
            'Update the Core payment policy before changing the merchant, store or product binding.'
          )
        )
      } else if (
        action === 'save' &&
        (acknowledged || ![400, 409, 422].includes(problem.status))
      ) {
        // An uncertain write is read back once. Never replay a configuration write.
        try {
          const refreshed = await readConfiguration(scope, controller.signal)
          if (!active()) return
          const actual = selectedBinding(refreshed, selectedID)
          client.setQueryData(queryKey, refreshed)
          setSelectedID(bindingID(actual))
          setReturnURL(actual.return_url)
          setPrivateKey('')
          setMessage(
            problem.code === 'PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN'
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
      } else {
        setMessage(
          problem.status === 409
            ? t(
                'The payment configuration changed on the server. Reload the service configuration before trying again.'
              )
            : t(
                'The payment request failed. Check the credentials, return URL and service status.'
              )
        )
      }
    } finally {
      if (inFlight.current === controller) inFlight.current = null
      if (active()) setBusy(null)
    }
  }

  return (
    <div className='space-y-10'>
      {configuration.bindings.length > 1 && (
        <div className='max-w-3xl space-y-2'>
          <Label htmlFor='native-payment-binding'>
            {t('Payment connection')}
          </Label>
          <select
            id='native-payment-binding'
            value={selectedID}
            disabled={busy !== null || dirty}
            aria-describedby='native-payment-binding-help'
            className='border-input bg-background block min-h-11 w-full rounded-md border px-3 text-sm'
            onChange={(event) => {
              if (dirty || inFlight.current) return
              const next = selectedBinding(
                configuration,
                Number(event.target.value)
              )
              setSelectedID(bindingID(next))
              setReturnURL(next.return_url)
              setPrivateKey('')
              setCatalog(null)
              setPair(null)
              setConfirmPair(false)
              setMessage('')
              setSaved(false)
            }}
          >
            {configuration.bindings.map((binding) => (
              <option key={bindingID(binding)} value={bindingID(binding)}>
                {t('{{environment}} · {{currencies}} · Channel #{{id}}', {
                  environment:
                    binding.environment === 'live'
                      ? t('Production')
                      : t('Test environment'),
                  currencies: [
                    ...new Set(
                      binding.channels.map((channel) => channel.currency)
                    ),
                  ].join(' / '),
                  id: bindingID(binding),
                })}
              </option>
            ))}
          </select>
          <p
            id='native-payment-binding-help'
            className='text-muted-foreground text-xs'
          >
            {dirty
              ? t(
                  'Save or discard changes before switching payment connections.'
                )
              : t(
                  'Choose the payment connection to review and edit. Changes apply to its listed currency channels.'
                )}
          </p>
        </div>
      )}
      <section className='space-y-5' aria-labelledby='payment-binding-heading'>
        <div className='flex flex-wrap items-start justify-between gap-4'>
          <div className='space-y-1'>
            <h2 id='payment-binding-heading' className='text-lg font-semibold'>
              Pancake
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
        <dl className='bg-muted/30 grid gap-x-8 gap-y-5 rounded-xl p-5 sm:grid-cols-2'>
          {[
            [t('Merchant ID'), current.merchant_id],
            [t('Store ID'), current.store_id],
            [t('Product ID'), current.product_id],
            [
              t('Currencies'),
              [
                ...new Set(current.channels.map((channel) => channel.currency)),
              ].join(' · '),
            ],
          ].map(([label, value]) => (
            <div key={label} className='min-w-0 space-y-1'>
              <dt className='text-muted-foreground text-xs'>{label}</dt>
              <dd className='font-mono text-sm break-all'>{value}</dd>
            </div>
          ))}
          <div className='space-y-1'>
            <dt className='text-muted-foreground text-xs'>
              {t('Private key')}
            </dt>
            <dd className='flex items-center gap-2 text-sm'>
              <ShieldCheck className='text-muted-foreground size-4' />
              {current.private_key_configured
                ? t('Configured')
                : t('Not configured')}
            </dd>
          </div>
          <div className='space-y-1'>
            <dt className='text-muted-foreground text-xs'>
              {t('Payment channels')}
            </dt>
            <dd className='text-sm'>
              {current.channels
                .map(
                  (channel) =>
                    `${channel.currency} #${channel.id} · ${channel.enabled ? t('Enabled') : t('Disabled')}`
                )
                .join(' / ')}
            </dd>
          </div>
        </dl>
        <p className='text-muted-foreground max-w-3xl text-sm'>
          {t(
            'Merchant, store and product IDs are bound to the Core payment policy. Changing that binding requires a coordinated host configuration update.'
          )}
        </p>
      </section>

      <form
        className='max-w-3xl space-y-6'
        onSubmit={(event) => {
          event.preventDefault()
          if (dirty) void run('save')
        }}
      >
        <h2 className='text-base font-semibold'>{t('Checkout settings')}</h2>
        <div className='space-y-2'>
          <Label htmlFor='native-payment-return-url'>
            {t('Payment return URL')}
          </Label>
          <Input
            id='native-payment-return-url'
            type='url'
            value={returnURL}
            disabled={busy !== null}
            onChange={(event) => {
              setReturnURL(event.target.value)
              setSaved(false)
            }}
            placeholder='https://example.com/wallet'
            autoComplete='off'
          />
          <p className='text-muted-foreground text-xs'>
            {t(
              'Customers return here after checkout. Leave empty to use the provider default.'
            )}
          </p>
        </div>
        <div className='space-y-2'>
          <Label htmlFor='native-payment-private-key'>
            {t('Replace private key')}
          </Label>
          <Textarea
            id='native-payment-private-key'
            value={privateKey}
            disabled={busy !== null}
            onChange={(event) => {
              setPrivateKey(event.target.value)
              setSaved(false)
            }}
            rows={4}
            autoComplete='off'
            autoCorrect='off'
            spellCheck={false}
            className='font-mono text-xs'
            aria-describedby='native-payment-key-help'
          />
          <p
            id='native-payment-key-help'
            className='text-muted-foreground text-xs'
          >
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
          <p role='status' className='flex items-center gap-2 text-sm'>
            <Check className='size-4' />
            {t('Payment settings saved and read back from the server.')}
          </p>
        )}
        <div className='flex flex-wrap gap-3'>
          <Button type='submit' disabled={!dirty || busy !== null}>
            {busy === 'save' ? t('Saving...') : t('Save payment settings')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={!dirty || busy !== null}
            onClick={() => {
              setReturnURL(current.return_url)
              setPrivateKey('')
              setMessage('')
              setSaved(false)
            }}
          >
            {t('Discard changes')}
          </Button>
        </div>
      </form>

      <section className='space-y-4' aria-labelledby='payment-catalog-heading'>
        <div className='flex flex-wrap items-center justify-between gap-4'>
          <div className='space-y-1'>
            <h2
              id='payment-catalog-heading'
              className='text-base font-semibold'
            >
              {t('Provider catalog')}
            </h2>
            <p className='text-muted-foreground max-w-2xl text-sm'>
              {t(
                'Check stores and products with the configured key, or the replacement key entered above. Checking does not save the replacement key.'
              )}
            </p>
          </div>
          <Button
            type='button'
            variant='outline'
            disabled={busy !== null}
            onClick={() => {
              void run('catalog')
            }}
          >
            <RefreshCw
              className={`size-4 ${busy === 'catalog' ? 'animate-spin' : ''}`}
            />
            {t('Load provider catalog')}
          </Button>
        </div>
        {catalog &&
          (catalog.stores.length === 0 ? (
            <p className='text-muted-foreground py-4 text-sm'>
              {t('No stores found in this merchant account.')}
            </p>
          ) : (
            <div className='space-y-3'>
              {catalog.stores.map((store) => (
                <div
                  key={store.id}
                  className='bg-muted/30 space-y-3 rounded-xl p-5'
                >
                  <div>
                    <p className='font-medium'>{store.name || store.id}</p>
                    <p className='text-muted-foreground font-mono text-xs break-all'>
                      {store.id}
                    </p>
                  </div>
                  <div className='grid gap-3 sm:grid-cols-2'>
                    {(
                      [
                        [t('One-time products'), store.onetimeProducts],
                        [
                          t('Subscription products'),
                          store.subscriptionProducts,
                        ],
                      ] as const
                    ).map(([title, products]) => (
                      <div key={title} className='space-y-2'>
                        <h3 className='text-muted-foreground text-xs font-medium'>
                          {title}
                        </h3>
                        {products.length === 0 ? (
                          <p className='text-muted-foreground text-sm'>
                            {t('No active products')}
                          </p>
                        ) : (
                          products.map((product) => (
                            <div key={product.id}>
                              <p className='text-sm'>
                                {product.name || product.id}
                              </p>
                              <p className='text-muted-foreground font-mono text-xs break-all'>
                                {product.id}
                              </p>
                            </div>
                          ))
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          ))}
        <details className='pt-3'>
          <summary className='cursor-pointer text-sm font-medium'>
            {t('Prepare a new store and product')}
          </summary>
          <div className='mt-4 max-w-3xl space-y-4'>
            <p className='text-muted-foreground text-sm'>
              {t(
                'This creates provider objects for the current merchant. The new IDs remain inactive until the host configuration and Core payment policy are updated together.'
              )}
            </p>
            <Button
              variant='outline'
              disabled={busy !== null}
              onClick={() => setConfirmPair(true)}
            >
              {t('Create store and product')}
            </Button>
            {pair && (
              <div
                role='status'
                className='bg-muted/30 space-y-2 rounded-lg p-4 text-sm'
              >
                <p className='font-medium'>
                  {pair.orphan_store
                    ? t('Product setup incomplete')
                    : t('Provider objects created; activation required')}
                </p>
                <p className='font-mono break-all'>
                  {t('Store ID')}: {pair.store_id}
                </p>
                {pair.product_id && (
                  <p className='font-mono break-all'>
                    {t('Product ID')}: {pair.product_id}
                  </p>
                )}
              </div>
            )}
          </div>
        </details>
      </section>
      <ConfirmDialog
        open={confirmPair}
        onOpenChange={setConfirmPair}
        title={t('Create store and product')}
        desc={t(
          'Create a store and wallet-credit product at the payment provider? Your active payment binding will require a separate host and Core policy update.'
        )}
        confirmText={t('Create provider objects')}
        isLoading={busy === 'pair'}
        handleConfirm={() => {
          void run('pair')
        }}
      />
    </div>
  )
}
