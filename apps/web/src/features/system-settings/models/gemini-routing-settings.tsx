/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import {
  getNativeConfiguration,
  readNativeRoutingScope,
  type NativeRoutingScope,
} from '@/features/channels/native-api'
import { useAuthStore } from '@/stores/auth-store'

import { ConfigurationAssistant } from '../configuration-assistant'

function GeminiRoutes({ scope }: { scope: NativeRoutingScope }) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: [
      'native-routing-configuration',
      scope.userId,
      scope.sessionId,
      scope.role,
    ],
    queryFn: () => getNativeConfiguration(scope),
    retry: false,
  })
  if (query.isError) {
    const status = isAxiosError(query.error) && query.error.response?.status
    return (
      <div role='alert' className='space-y-3'>
        <p>
          {status === 401 || status === 403
            ? t('Access denied')
            : t('Unable to load settings')}
        </p>
        <Button
          variant='outline'
          size='sm'
          onClick={() => void query.refetch()}
        >
          {t('Retry')}
        </Button>
      </div>
    )
  }
  if (!query.data) return <p role='status'>{t('Loading settings...')}</p>
  const { routing, bindings } = query.data
  const channels = routing.upstreams.filter(
    (channel) => channel.protocol === 'gemini'
  )
  return (
    <div className='space-y-8'>
      <p className='text-muted-foreground text-sm'>
        {t('Revision {{revision}}', { revision: routing.version })}
      </p>
      <div className='space-y-1 text-sm'>
        <p>
          {t('Source')} ·{' '}
          <code>{query.data.authority?.source ?? t('Unavailable')}</code>
        </p>
        {query.data.authority && (
          <details>
            <summary className='cursor-pointer'>{t('Details')}</summary>
            <dl className='mt-2 space-y-2 break-all'>
              <div>
                <dt>{t('Revision')} · Git</dt>
                <dd>
                  <code>
                    {query.data.authority.git_revision ?? t('Unavailable')}
                  </code>
                </dd>
              </div>
              <div>
                <dt>{t('Routing Reliability')} · SHA-256</dt>
                <dd>
                  <code>{query.data.authority.routing_sha256}</code>
                </dd>
              </div>
              <div>
                <dt>{t('Pricing')} · SHA-256</dt>
                <dd>
                  <code>{query.data.authority.pricing_sha256}</code>
                </dd>
              </div>
            </dl>
          </details>
        )}
      </div>
      <ConfigurationAssistant domain='gemini' revision={routing.version} />
      {!channels.length && <p>{t('No channels found')}</p>}
      {channels.map((channel) => {
        const mappings = routing.routes.flatMap((route) =>
          route.targets
            .filter((target) => target.upstream_id === channel.id)
            .map((target) => ({ route, target }))
        )
        return (
          <section key={channel.id} className='space-y-3'>
            <header className='flex flex-wrap items-baseline gap-3'>
              <h2 className='font-semibold'>
                {bindings.names[channel.id] || `#${channel.id}`}
              </h2>
              <a
                className='text-primary text-sm underline underline-offset-4'
                href={`/channels?edit=${channel.id}`}
              >
                {t('Configuration')}
              </a>
              <span className='text-muted-foreground text-sm'>
                {channel.enabled ? t('Enabled') : t('Disabled')}
              </span>
            </header>
            <p className='text-sm break-all'>
              {t('Base URL')} · <code>{channel.endpoint}</code>
            </p>
            {mappings.length ? (
              <ul className='divide-border/50 divide-y'>
                {mappings.map(({ route, target }) => (
                  <li
                    key={`${route.id}-${target.id}`}
                    className='grid gap-1 py-2 text-sm'
                  >
                    <span className='break-all'>
                      {route.model} → {target.upstream_model}
                    </span>
                    <span className='text-muted-foreground break-words'>
                      {t('Group')} · {route.group} ·{' '}
                      {t('Allowed client protocols')} ·{' '}
                      {target.client_protocols?.join(', ') ?? t('All')}
                    </span>
                    {(!route.enabled ||
                      !target.enabled ||
                      !routing.groups.some(
                        (g) => g.name === route.group && g.enabled
                      )) && <span>{t('Disabled')}</span>}
                  </li>
                ))}
              </ul>
            ) : (
              <p className='text-muted-foreground text-sm'>
                {t('No models found')}
              </p>
            )}
          </section>
        )
      })}
    </div>
  )
}

export function GeminiRoutingSettings() {
  const { t } = useTranslation()
  const scope = readNativeRoutingScope(useAuthStore((state) => state.auth))
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Gemini')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <a
          href='/channels'
          className='text-primary text-sm font-medium underline underline-offset-4'
        >
          {t('Channel Management')}
        </a>
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <div className='max-w-3xl space-y-6'>
          <p className='text-muted-foreground text-sm leading-relaxed'>
            {t(
              'The channel base URL determines the Gemini API version. Each mapping specifies its upstream model and allowed client protocols. Global safety and thinking policies are unavailable.'
            )}
          </p>
          {scope ? (
            <GeminiRoutes scope={scope} />
          ) : (
            <p role='alert'>{t('Access denied')}</p>
          )}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
