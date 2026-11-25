/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, type UseQueryResult } from '@tanstack/react-query'
import { Suspense, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Button } from '@/components/ui/button'
import {
  nativeUsersFailure,
  nativeUsersOptions,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { ConfigurationAssistant } from './configuration-assistant'
import {
  getAutoGroupOrder,
  getRequestLimits,
} from './request-limits/native-api'
import { lazyNamedSection } from './utils/lazy-section'
import { getSettingsErrorMessage } from './utils/settings-error-message'

export type CoreConfigurationDomain =
  | 'identity'
  | 'billing'
  | 'routing'
  | 'console'

const HumanGrantSection = lazyNamedSection(
  () => import('./auth/humangrant-section'),
  'HumanGrantSection'
)
const configDocs =
  'https://github.com/TokenNotIncluded/lmm-api-source/blob/701f65496e512b206bb9440020d8e1a3ebfb3dcc/'
function ConfigReference({
  label,
  path,
  children,
}: {
  label: string
  path: string
  children: ReactNode
}) {
  return (
    <Field label={label}>
      <a
        href={configDocs + path}
        className='text-primary break-words underline underline-offset-4'
      >
        <code>{children}</code>
      </a>
    </Field>
  )
}

function IdentityConfiguration() {
  const { t } = useTranslation()
  const [showGrant, setShowGrant] = useState(false)
  return (
    <div className='space-y-4'>
      <h3 className='font-medium'>{t('Configuration File')}</h3>
      <dl className='space-y-3'>
        <ConfigReference
          label={t('Basic Authentication')}
          path='config/core.example.toml'
        >
          identity.email_verification_enabled ·
          identity.email_domain_restriction_enabled ·
          identity.email_domain_whitelist
        </ConfigReference>
        <ConfigReference
          label={t('OAuth Integrations')}
          path='docs/core-identity.md'
        >
          identity.google · identity.github · identity.linuxdo ·
          identity.discord · identity.telegram
        </ConfigReference>
        <ConfigReference
          label={t('Custom OAuth')}
          path='docs/federated-auth.md'
        >
          identity.federated_auth · apply-federated-provider
        </ConfigReference>
        <ConfigReference
          label={t('Passkey')}
          path='docs/checkin-anti-automation.md'
        >
          identity.web.origin · identity.web.passkey_rp_id
        </ConfigReference>
        <ConfigReference
          label={t('Bot Protection')}
          path='config/core.example.toml'
        >
          identity.turnstile
        </ConfigReference>
        <ConfigReference label={t('SMTP Email')} path='docs/core-identity.md'>
          identity.email_config_file
        </ConfigReference>
        <ConfigReference
          label={t('IP access routing')}
          path='docs/network-access.md'
        >
          console.ip_access_routing_rules
        </ConfigReference>
        <ConfigReference
          label={t('HumanGrant')}
          path='services/core/src/config.rs'
        >
          identity.humangrant_enabled
        </ConfigReference>
      </dl>
      <details onToggle={(event) => setShowGrant(event.currentTarget.open)}>
        <summary className='cursor-pointer font-medium'>
          {t('HumanGrant')}
        </summary>
        {showGrant && (
          <div className='mt-3'>
            <Suspense
              fallback={<p role='status'>{t('Loading settings...')}</p>}
            >
              <HumanGrantSection readOnly />
            </Suspense>
          </div>
        )}
      </details>
    </div>
  )
}

const optionsResponse = z.object({
  success: z.literal(true),
  data: z.array(
    z.object({
      key: z.string(),
      value: z.string(),
      source: z.string().nullish(),
      revision: z.string().nullish(),
    })
  ),
})
const announcementList = z.array(
  z.object({
    id: z.number().int(),
    content: z.string(),
    publishDate: z.string(),
  })
)

function ReadResult<T>({
  query,
  children,
}: {
  query: UseQueryResult<T>
  children: (data: T) => ReactNode
}) {
  const { t } = useTranslation()
  if (query.isError) {
    const denied = nativeUsersFailure(query.error).kind === 'forbidden'
    return (
      <div role='alert' className='space-y-2'>
        <p>
          {denied
            ? t('Access denied')
            : query.error instanceof z.ZodError
              ? t('Unable to load settings')
              : getSettingsErrorMessage(
                  query.error,
                  t('Unable to load settings')
                )}
        </p>
        <Button
          type='button'
          size='sm'
          variant='outline'
          onClick={() => void query.refetch()}
        >
          {t('Retry')}
        </Button>
      </div>
    )
  }
  if (!query.isSuccess) return <p role='status'>{t('Loading settings...')}</p>
  return children(query.data)
}

function Field({
  label,
  field,
  origin,
  children,
}: {
  label: string
  field?: string
  origin?: { source?: string | null; revision?: string | null }
  children: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <div className='space-y-1'>
      <dt className='text-muted-foreground text-sm'>{label}</dt>
      <dd className='min-w-0 break-words whitespace-pre-wrap'>{children}</dd>
      {origin && (
        <dd className='text-muted-foreground text-xs'>
          {t('Source')} · <code>{origin.source || t('Unavailable')}</code> ·{' '}
          {t('Revision')} · <code>{origin.revision ?? t('Unavailable')}</code>
        </dd>
      )}
      {field && (
        <dd className='text-muted-foreground font-mono text-xs break-all'>
          {field}
        </dd>
      )}
    </div>
  )
}

function Announcements({ value }: { value?: string }) {
  const { t } = useTranslation()
  if (value === undefined) return t('Unavailable')
  let data: unknown
  try {
    data = JSON.parse(value)
  } catch {
    return t('Unavailable')
  }
  const items = announcementList.safeParse(data)
  if (!items.success) return t('Unavailable')
  if (!items.data.length) return t('No announcements at this time')
  return (
    <ul className='space-y-2'>
      {items.data.map((item) => (
        <li key={item.id}>
          <span>{item.content}</span>{' '}
          <time
            dateTime={item.publishDate}
            className='text-muted-foreground text-sm'
          >
            {item.publishDate}
          </time>
        </li>
      ))}
    </ul>
  )
}

function OptionProjection({
  scope,
  domain,
}: {
  scope: NativeUsersScope
  domain: Exclude<CoreConfigurationDomain, 'routing'>
}) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['core-configuration', scope.userId, scope.sessionId, 'options'],
    queryFn: async ({ signal }) => {
      const response = await api.get<unknown>(
        '/api/option/',
        nativeUsersOptions(scope, signal)
      )
      return optionsResponse.parse(response.data).data
    },
    retry: false,
  })
  return (
    <ReadResult query={query}>
      {(options) => {
        const origins = new Map(options.map((item) => [item.key, item]))
        const values = new Map(options.map(({ key, value }) => [key, value]))
        function display(key: string) {
          const value = values.get(key)
          if (value === undefined) return t('Unavailable')
          if (value === '') return t('Not configured')
          return value
        }
        function toggle(key: string) {
          const value = values.get(key)
          if (value === 'true') return t('Enabled')
          if (value === 'false') return t('Disabled')
          return t('Unavailable')
        }
        return (
          <dl className='space-y-4'>
            {domain === 'identity' && (
              <>
                <Field
                  label={t('Registration Enabled')}
                  field='identity.registration_enabled'
                  origin={origins.get('RegisterEnabled')}
                >
                  {toggle('RegisterEnabled')}
                </Field>
                <Field
                  label={t('Password Registration')}
                  field='identity.password_register_enabled'
                  origin={origins.get('PasswordRegisterEnabled')}
                >
                  {toggle('PasswordRegisterEnabled')}
                </Field>
                <Field
                  label={t('OAuth Registration')}
                  field='identity.oauth_register_enabled'
                  origin={origins.get('OAuthRegisterEnabled')}
                >
                  {toggle('OAuthRegisterEnabled')}
                </Field>
                <Field
                  label={t('Disabled registration methods')}
                  field='identity.registration_disabled_methods'
                  origin={origins.get('RegistrationDisabledMethods')}
                >
                  {display('RegistrationDisabledMethods')}
                </Field>
                <Field
                  label={t('IP access routing')}
                  field='console.ip_access_routing_rules'
                  origin={origins.get('IPAccessRoutingRules')}
                >
                  {display('IPAccessRoutingRules')}
                </Field>
              </>
            )}
            {domain === 'billing' && (
              <>
                <Field
                  label={t('USD Exchange Rate')}
                  field='console.usd_exchange_rate'
                  origin={origins.get('USDExchangeRate')}
                >
                  {display('USDExchangeRate')}
                </Field>
                <ConfigReference
                  label={t('Core payment configuration')}
                  path='config/core-payments.example.toml'
                >
                  payments.config_file
                </ConfigReference>
              </>
            )}
            {domain === 'console' && (
              <>
                <Field
                  label={t('Server Address')}
                  field='identity.web.origin'
                  origin={origins.get('ServerAddress')}
                >
                  {display('ServerAddress')}
                </Field>
                <Field
                  label={t('System Notice')}
                  field='console.notice'
                  origin={origins.get('Notice')}
                >
                  {display('Notice')}
                </Field>
                <Field
                  label={t('Announcements')}
                  field='console.announcements'
                  origin={
                    origins.get('console_setting.announcements') ??
                    origins.get('Announcements')
                  }
                >
                  <Announcements
                    value={
                      values.get('console_setting.announcements') ??
                      values.get('Announcements')
                    }
                  />
                </Field>
              </>
            )}
          </dl>
        )
      }}
    </ReadResult>
  )
}

function RoutingProjection({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const prefix = ['core-configuration', scope.userId, scope.sessionId]
  const limits = useQuery({
    queryKey: [...prefix, 'request-limits'],
    queryFn: () => getRequestLimits(scope),
    retry: false,
  })
  const groups = useQuery({
    queryKey: [...prefix, 'auto-groups'],
    queryFn: () => getAutoGroupOrder(scope),
    retry: false,
  })
  return (
    <div className='space-y-5'>
      <section className='space-y-2'>
        <h3 className='font-medium'>{t('Request Limits')}</h3>
        <ReadResult query={limits}>
          {({ revision, policy }) => (
            <dl className='space-y-3'>
              <Field label={t('Revision')}>{revision}</Field>
              <Field label={t('Rate Limiting')}>
                {policy.enabled ? t('Enabled') : t('Disabled')}
              </Field>
              <Field label={t('Fixed window (seconds)')}>
                {policy.window_seconds}
              </Field>
              <Field label={t('Requests per user per window')}>
                {policy.user_limit ?? t('No limit')}
              </Field>
              <Field label={t('Shared group request limits')}>
                {Object.keys(policy.group_limits).length ? (
                  <dl>
                    {Object.entries(policy.group_limits).map(
                      ([group, limit]) => (
                        <div className='flex gap-3' key={group}>
                          <dt>{group}</dt>
                          <dd>{limit}</dd>
                        </div>
                      )
                    )}
                  </dl>
                ) : (
                  t('No group limits configured.')
                )}
              </Field>
            </dl>
          )}
        </ReadResult>
      </section>
      <section className='space-y-2'>
        <h3 className='font-medium'>{t('Global Auto group order')}</h3>
        <ReadResult query={groups}>
          {({ revision, groups: ordered }) => (
            <div className='space-y-2'>
              <p className='text-muted-foreground text-sm'>
                {t('Revision {{revision}}', { revision })}
              </p>
              {ordered.length ? (
                <ol className='list-decimal space-y-1 ps-5'>
                  {ordered.map((group) => (
                    <li key={group}>{group}</li>
                  ))}
                </ol>
              ) : (
                <p>{t('Not configured')}</p>
              )}
            </div>
          )}
        </ReadResult>
      </section>
      <section className='space-y-2'>
        <h3 className='font-medium'>{t('Routing Reliability')}</h3>
        <a
          className='text-primary underline underline-offset-4'
          href='/channels'
        >
          {t('Channel Management')}
        </a>
        <dl>
          <ConfigReference
            label={t('Configuration File')}
            path='docs/reports/native-channel-routing-20261118.md'
          >
            model.config_file · routes[].failover.max_attempts ·
            routes[].failover.retry_on
          </ConfigReference>
        </dl>
      </section>
      <div>
        <p className='text-muted-foreground text-sm'>
          {t('Request Limits')} · {t('Global Auto group order')}
        </p>
        <code className='break-all'>
          lmm-core --config /etc/lmm.conf.d/core.toml apply-routing-policy &lt;
          routing-policy.toml
        </code>
      </div>
    </div>
  )
}

export function CoreConfigurationSection({
  domain,
}: {
  domain: CoreConfigurationDomain
}) {
  const { t } = useTranslation()
  const scope = readNativeUsersScope(useAuthStore((state) => state.auth))
  if (!scope || scope.role !== ROLE.SUPER_ADMIN) {
    return <p role='alert'>{t('Super administrator access required.')}</p>
  }
  const headings = {
    identity: t('Identity'),
    console: t('Console'),
    billing: t('Billing'),
    routing: t('Routing Reliability'),
  }
  return (
    <section className='space-y-5' aria-label={headings[domain]}>
      <h2 className='text-lg font-semibold'>{headings[domain]}</h2>
      <ConfigurationAssistant domain={domain} />
      {domain === 'routing' ? (
        <RoutingProjection scope={scope} />
      ) : (
        <>
          <div className='space-y-3'>
            <h3 className='font-medium'>
              {t('Current settings')} · {t('Read only')}
            </h3>
            <OptionProjection scope={scope} domain={domain} />
          </div>
          {domain === 'identity' && <IdentityConfiguration />}
          {domain !== 'identity' && (
            <a
              href={`${configDocs}docs/core-console-settings.md`}
              className='text-primary text-sm underline underline-offset-4'
            >
              {t('Configuration File')}
            </a>
          )}
        </>
      )}
    </section>
  )
}
