/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useQuery } from '@tanstack/react-query'
import { ArrowUpRight, Check, ChevronDown, Copy, KeyRound } from 'lucide-react'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useApiBaseUrl } from '@/hooks/use-api-base-url'
import { useCopyToClipboard } from '@/hooks/use-copy-to-clipboard'
import {
  OAUTH_DISCOVERY_PATH,
  readPublicOAuthMetadata,
} from '@/lib/public-oauth-metadata'

import {
  LMM_SOURCE,
  buildOAuthPrompt,
  buildPricingExample,
  buildPricingPrompt,
} from './integration-prompts'
import { RequestBuilder } from './request-builder'

export function IntegrationPrompt({
  prompt,
  label,
}: {
  prompt: string
  label: string
}) {
  const { t } = useTranslation()
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [failed, setFailed] = useState(false)
  const details = useRef<HTMLDetailsElement>(null)
  const field = useRef<HTMLTextAreaElement>(null)
  const copy = async () => {
    const copied = await copyToClipboard(prompt)
    setFailed(!copied)
    if (!copied) {
      if (details.current) details.current.open = true
      field.current?.focus()
      field.current?.select()
    }
  }
  return (
    <div className='dev-integration-prompt'>
      <div className='dev-prompt-heading'>
        <Button variant='outline' onClick={() => void copy()}>
          <Copy aria-hidden='true' />
          {copiedText === prompt ? t('Copied') : label}
        </Button>
        <p>
          {t(
            'Prompts contain technical instructions in English, never credentials.'
          )}
        </p>
      </div>
      {failed && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Copy failed. Select and copy the prompt below.')}
        </p>
      )}
      <details ref={details}>
        <summary>
          {t('Preview prompt')}
          <ChevronDown aria-hidden='true' />
        </summary>
        <textarea
          ref={field}
          readOnly
          value={prompt}
          rows={12}
          aria-label={`${t('Integration prompt')}: ${label}`}
          onFocus={(event) => event.currentTarget.select()}
        />
      </details>
      <span role='status' className='sr-only'>
        {copiedText === prompt ? t('Copied to clipboard') : ''}
      </span>
    </div>
  )
}

function PricingExample({ rootUrl }: { rootUrl: string }) {
  const pricingExample = buildPricingExample(rootUrl)
  const { t } = useTranslation()
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [failed, setFailed] = useState(false)
  const copy = async () => setFailed(!(await copyToClipboard(pricingExample)))
  const copied = copiedText === pricingExample

  return (
    <div className='dev-reference-code'>
      <div className='dev-reference-code-head'>
        <span>JavaScript</span>
        <Button
          type='button'
          variant='ghost'
          aria-label={`${t('Copy')}: ${t('Fetch pricing from your backend.')}`}
          onClick={() => void copy()}
        >
          {copied ? <Check aria-hidden='true' /> : <Copy aria-hidden='true' />}
          <span aria-live='polite'>{copied ? t('Copied') : t('Copy')}</span>
        </Button>
      </div>
      <pre tabIndex={0} aria-label={t('Fetch pricing from your backend.')}>
        <code>{pricingExample}</code>
      </pre>
      {failed && (
        <p role='alert' className='text-destructive px-4 pb-4 text-sm'>
          {t('Copy failed')}
        </p>
      )}
    </div>
  )
}

function Endpoint({
  method,
  path,
  label,
}: {
  method: string
  path: string
  label: string
}) {
  return (
    <div className='dev-endpoint'>
      <dt>{label}</dt>
      <dd>
        <span className='dev-http-method'>{method}</span>
        <code>{path}</code>
      </dd>
    </div>
  )
}

function ConnectionAddress() {
  const { t } = useTranslation()
  const { baseUrl, loading } = useApiBaseUrl()
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [failed, setFailed] = useState(false)
  const field = useRef<HTMLInputElement>(null)
  const copy = async () => {
    if (!baseUrl) return
    const copied = await copyToClipboard(baseUrl)
    setFailed(!copied)
    if (!copied) {
      field.current?.focus()
      field.current?.select()
    }
  }
  const copied = Boolean(baseUrl && copiedText === baseUrl)

  return (
    <div className='dev-connection'>
      <div>
        <label htmlFor='dev-base-url'>{t('Base URL')}</label>
        <p>{t('Use this base URL in OpenAI-compatible clients.')}</p>
      </div>
      <div className='dev-connection-value'>
        <input
          ref={field}
          id='dev-base-url'
          readOnly
          value={baseUrl ?? ''}
          placeholder={t(loading ? 'Loading...' : 'API address unavailable')}
          onFocus={(event) => event.currentTarget.select()}
        />
        <Button
          variant='ghost'
          size='icon'
          disabled={!baseUrl}
          aria-label={t('Copy Base URL')}
          onClick={() => void copy()}
        >
          {copied ? <Check aria-hidden='true' /> : <Copy aria-hidden='true' />}
        </Button>
        <span role='status' className='sr-only'>
          {copied ? t('Copied to clipboard') : ''}
        </span>
      </div>
      {failed && <p role='alert'>{t('Copy failed')}</p>}
    </div>
  )
}

export function DevelopersPage() {
  const { t } = useTranslation()
  const { rootUrl, loading } = useApiBaseUrl()
  const oauth = useQuery({
    queryKey: ['public-oauth-metadata'],
    queryFn: ({ signal }) => readPublicOAuthMetadata(signal),
    staleTime: 60_000,
  })
  return (
    <main className='developers-page'>
      <header className='developers-hero'>
        <div>
          <h1>{t('Developers')}</h1>
          <p className='dev-introduction'>
            {t(
              'Configure your client, generate a request, and connect to LMM API.'
            )}
          </p>
        </div>
        <Button render={<a href='/keys' />}>
          <KeyRound aria-hidden='true' />
          {t('Manage API Keys')}
          <ArrowUpRight aria-hidden='true' />
        </Button>
      </header>

      <div className='dev-page-layout'>
        <nav className='dev-page-nav' aria-label={t('Integration guide')}>
          <p className='dev-eyebrow'>{t('API reference')}</p>
          <a className='dev-page-link' href='#request-builder'>
            {t('Request builder')}
          </a>
          <a className='dev-page-link' href='#pricing-api'>
            {t('Pricing API')}
          </a>
          <a className='dev-page-link' href='#oauth-integration'>
            {t('OAuth integration')}
          </a>
          <div className='dev-nav-related'>
            <a className='dev-page-link' href='/pricing'>
              {t('Models')}
              <ArrowUpRight aria-hidden='true' />
            </a>
            <a className='dev-page-link' href='/webmcp'>
              WebMCP
              <ArrowUpRight aria-hidden='true' />
            </a>
          </div>
        </nav>

        <div className='dev-page-content'>
          <ConnectionAddress />
          <section id='request-builder' aria-labelledby='request-builder-title'>
            <div className='dev-section-heading'>
              <h2 id='request-builder-title'>{t('Request builder')}</h2>
              <p>
                {t(
                  'Pick a protocol and language. The snippet updates as you type and is never sent.'
                )}
              </p>
            </div>
            <RequestBuilder />
          </section>

          <section
            id='pricing-api'
            aria-labelledby='pricing-api-title'
            className='dev-reference-section'
          >
            <div className='dev-section-heading'>
              <h2 id='pricing-api-title'>{t('Pricing API')}</h2>
              <p>{t('Public prices follow the site visibility settings.')}</p>
            </div>
            <dl className='dev-endpoints'>
              <Endpoint
                method='GET'
                path='/core/v1/pricing/public'
                label={t('Public pricing')}
              />
              <Endpoint
                method='GET'
                path='/core/v1/pricing/session'
                label={t('Account catalog')}
              />
              <Endpoint
                method='GET'
                path='/api/oauth2/catalog'
                label={t('OAuth integration')}
              />
            </dl>
            <p className='dev-reference-note'>
              {t('Missing prices are unknown, not zero.')}
            </p>
            <h3 className='dev-reference-subtitle'>
              {t('Fetch pricing from your backend.')}
            </h3>
            {rootUrl ? (
              <PricingExample rootUrl={rootUrl} />
            ) : (
              <p role='status'>
                {t(loading ? 'Loading...' : 'API address unavailable')}
              </p>
            )}
            <a
              className='dev-source-link'
              href={`${LMM_SOURCE}/blob/main/services/core/src/model_http/catalog.rs`}
              target='_blank'
              rel='noopener noreferrer'
            >
              {t('Read the source contract')}
              <ArrowUpRight aria-hidden='true' />
            </a>
            {rootUrl ? (
              <IntegrationPrompt
                prompt={buildPricingPrompt(rootUrl)}
                label={t('Copy pricing prompt')}
              />
            ) : null}
          </section>

          <section
            id='oauth-integration'
            aria-labelledby='oauth-integration-title'
            className='dev-reference-section'
          >
            <div className='dev-section-heading'>
              <h2 id='oauth-integration-title'>{t('OAuth integration')}</h2>
              <p>{t('Authorization Code with PKCE (S256).')}</p>
            </div>
            <h3 className='dev-reference-subtitle'>
              {t('Registered native clients')}
            </h3>
            <dl className='dev-endpoints'>
              <Endpoint
                method='GET'
                path='/.well-known/oauth-authorization-server'
                label={t('Discovery')}
              />
              <Endpoint
                method='GET'
                path='/api/oauth2/authorize'
                label={t('Authorization')}
              />
              <Endpoint
                method='POST'
                path='/api/oauth2/token'
                label={t('Token exchange')}
              />
              <Endpoint
                method='POST'
                path='/api/oauth2/revoke'
                label={t('Token revocation')}
              />
            </dl>
            <h3 className='dev-reference-subtitle'>
              {t('Approved client settings')}
            </h3>
            <p className='dev-reference-note'>
              {t(
                'Client ID and scope profile must come from the server registry. Never reuse another application’s client ID.'
              )}
            </p>
            <div className='dev-reference-links'>
              <a
                className='dev-source-link'
                href={OAUTH_DISCOVERY_PATH}
                target='_blank'
                rel='noopener noreferrer'
              >
                {t('Discovery')}
                <ArrowUpRight aria-hidden='true' />
              </a>
              <a
                className='dev-source-link'
                href={`${LMM_SOURCE}/blob/main/services/core/src/oauth2_http.rs`}
                target='_blank'
                rel='noopener noreferrer'
              >
                {t('Read the source contract')}
                <ArrowUpRight aria-hidden='true' />
              </a>
            </div>
            {oauth.data && (
              <IntegrationPrompt
                prompt={buildOAuthPrompt(oauth.data.issuer)}
                label={t('Copy OAuth prompt')}
              />
            )}
          </section>
        </div>
      </div>
    </main>
  )
}
