/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useQuery } from '@tanstack/react-query'
import { Check, Copy, KeyRound } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { getPricing, pricingQueryKey } from '@/features/pricing/api'
import { useApiBaseUrl } from '@/hooks/use-api-base-url'
import { useCopyToClipboard } from '@/hooks/use-copy-to-clipboard'
import { useAuthStore } from '@/stores/auth-store'

import { buildRequestBody, buildRequestSnippet } from './request-snippet'

import './request-builder.css'

type Protocol = 'chat' | 'messages' | 'gemini'
type Language = 'curl' | 'javascript' | 'python'

const PROTOCOLS: readonly {
  id: Protocol
  label: string
  path: string
  shape: 'openai' | 'anthropic' | 'gemini'
}[] = [
  {
    id: 'chat',
    label: 'Chat Completions',
    path: '/v1/chat/completions',
    shape: 'openai',
  },
  {
    id: 'messages',
    label: 'Claude Messages',
    path: '/v1/messages',
    shape: 'anthropic',
  },
  {
    id: 'gemini',
    label: 'Gemini',
    path: '/v1beta/models/{model}:generateContent',
    shape: 'gemini',
  },
]

const LANGUAGES: readonly { id: Language; label: string }[] = [
  { id: 'curl', label: 'cURL' },
  { id: 'javascript', label: 'JavaScript' },
  { id: 'python', label: 'Python' },
]

const MODEL_OPTIONS: Readonly<
  Record<
    Protocol,
    {
      endpoint: string
    }
  >
> = {
  chat: {
    endpoint: 'openai',
  },
  messages: {
    endpoint: 'anthropic',
  },
  gemini: {
    endpoint: 'gemini',
  },
}

function sortModels(models: string[]): string[] {
  return models.sort((left, right) => {
    return left.localeCompare(right, undefined, {
      numeric: true,
      sensitivity: 'base',
    })
  })
}

/**
 * Build a request against the configured public API without sending the prompt. Model
 * choices come from the live public pricing catalogue, filtered by endpoint
 * compatibility, so the examples do not age into hard-coded legacy models.
 */
export function RequestBuilder() {
  const { t } = useTranslation()
  const { rootUrl, loading } = useApiBaseUrl()
  const { user, session } = useAuthStore((state) => state.auth)
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [protocol, setProtocol] = useState<Protocol>('chat')
  const [language, setLanguage] = useState<Language>('curl')
  const [enteredModel, setModel] = useState<string | null>(null)
  const [prompt, setPrompt] = useState('Explain LMM in one sentence.')
  const [copyFailed, setCopyFailed] = useState(false)
  const code = useRef<HTMLPreElement>(null)

  const pricingQuery = useQuery({
    queryKey: pricingQueryKey(user?.id, session?.sid),
    queryFn: ({ signal }) =>
      getPricing(signal, { userId: user?.id, sessionId: session?.sid }),
    staleTime: 5 * 60_000,
    retry: false,
  })

  const modelOptions = useMemo(() => {
    if (pricingQuery.isError) return []
    const config = MODEL_OPTIONS[protocol]
    const liveModels =
      pricingQuery.data?.data
        .filter(
          (item) =>
            item.runtime_state?.status !== 'unavailable' &&
            item.supported_endpoint_types?.includes(config.endpoint)
        )
        .map((item) => item.model_name.trim())
        .filter(Boolean) ?? []

    const uniqueModels = [...new Set(liveModels)]
    return sortModels(uniqueModels)
  }, [pricingQuery.data?.data, pricingQuery.isError, protocol])

  const model = enteredModel ?? modelOptions[0] ?? ''

  const active = PROTOCOLS.find((item) => item.id === protocol) ?? PROTOCOLS[0]
  const url = rootUrl
    ? `${rootUrl}${active.path.replace('{model}', encodeURIComponent(model || 'MODEL'))}`
    : null
  const body = useMemo(
    () => buildRequestBody(active.shape, model || 'MODEL', prompt || 'Hello'),
    [active.shape, model, prompt]
  )
  const snippet = useMemo(
    () => (url ? buildRequestSnippet(language, url, body) : ''),
    [language, url, body]
  )
  const copied = copiedText === snippet

  const copy = async () => {
    if (!url) return
    const success = await copyToClipboard(snippet)
    setCopyFailed(!success)
    if (!success && code.current) {
      code.current.focus()
      const range = document.createRange()
      range.selectNodeContents(code.current)
      const selection = window.getSelection()
      selection?.removeAllRanges()
      selection?.addRange(range)
    }
  }

  return (
    <div className='dev-builder'>
      <div className='dev-protocols' role='group' aria-label={t('Protocol')}>
        {PROTOCOLS.map((item) => (
          <button
            key={item.id}
            type='button'
            aria-pressed={protocol === item.id}
            onClick={() => {
              setProtocol(item.id)
              setModel(null)
              setCopyFailed(false)
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div className='dev-builder-workspace'>
        <div className='dev-builder-fields'>
          <div className='dev-builder-field'>
            <label className='dev-builder-label' htmlFor='dev-builder-model'>
              {t('Model')}
            </label>
            <input
              id='dev-builder-model'
              list='dev-builder-model-options'
              value={model}
              placeholder='MODEL'
              onChange={(event) => setModel(event.target.value)}
              aria-describedby='dev-model-help'
              className='dev-builder-input'
            />
            <datalist id='dev-builder-model-options'>
              {modelOptions.map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </datalist>
            <p id='dev-model-help' className='dev-field-help' role='status'>
              {pricingQuery.isError
                ? t(
                    'Model catalog unavailable. You can still enter a model ID.'
                  )
                : pricingQuery.isFetching
                  ? t('Loading...')
                  : t('Choose from the catalog or enter a model ID.')}
            </p>
          </div>
          <div className='dev-builder-field'>
            <label className='dev-builder-label' htmlFor='dev-builder-prompt'>
              {t('Prompt')}
            </label>
            <textarea
              id='dev-builder-prompt'
              value={prompt}
              maxLength={200}
              rows={4}
              onChange={(event) => setPrompt(event.target.value)}
              className='dev-builder-input dev-builder-prompt'
            />
          </div>
          <div className='dev-key-note'>
            <KeyRound aria-hidden='true' />
            <p>{t('Keep LMM_API_KEY in your server environment.')}</p>
          </div>
        </div>
        <div className='dev-builder-output'>
          <div className='dev-builder-output-head'>
            <div
              role='group'
              aria-label={t('Language')}
              className='dev-builder-tabs'
            >
              {LANGUAGES.map((item) => (
                <button
                  key={item.id}
                  type='button'
                  aria-pressed={language === item.id}
                  onClick={() => {
                    setLanguage(item.id)
                    setCopyFailed(false)
                  }}
                  className='dev-builder-tab'
                >
                  {item.label}
                </button>
              ))}
            </div>
            <Button
              type='button'
              variant='ghost'
              size='sm'
              aria-label={`${t('Copy')}: ${active.label} · ${language}`}
              disabled={!url}
              onClick={() => void copy()}
              className='dev-copy-request'
            >
              {copied ? (
                <Check aria-hidden='true' />
              ) : (
                <Copy aria-hidden='true' />
              )}
              <span aria-live='polite'>{copied ? t('Copied') : t('Copy')}</span>
            </Button>
          </div>
          <div className='dev-request-address'>
            <span className='dev-http-method'>POST</span>
            <code>
              {url ?? t(loading ? 'Loading...' : 'API address unavailable')}
            </code>
          </div>
          <pre
            ref={code}
            tabIndex={0}
            aria-label={`${active.label} · ${language}`}
          >
            <code>{snippet}</code>
          </pre>
          {copyFailed && (
            <p role='alert' className='dev-copy-error'>
              {t('Copy failed. The code is selected for manual copying.')}
            </p>
          )}
        </div>
      </div>
      <p className='dev-builder-footnote'>
        {t(
          'Nothing is sent. Copy the request into your own client and use your own key.'
        )}
      </p>
    </div>
  )
}
