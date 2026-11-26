/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isLosslessNumber } from 'lossless-json'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  NativePolicySection,
  type PolicyFormProps,
} from '@/features/system-settings/models/native-policy-section'

import { parseNativeDocument, stringifyNativeDocument } from '../../native-api'

type ChannelRules = Record<
  string,
  {
    parameters: Record<string, unknown>
    headers: Record<string, string>
    status_codes: Record<string, number>
  }
>

const empty = { parameters: {}, headers: {}, status_codes: {} }

function checkedOverrides(
  id: number,
  parameterText: string,
  headerText: string,
  statusText: string
): ChannelRules[string] {
  const parameters = parseNativeDocument(parameterText, [
    'channel_overrides',
    String(id),
    'parameters',
  ])
  const headers = parseNativeDocument(headerText)
  const statusCodes = parseNativeDocument(statusText)
  const validParameters = Object.entries(parameters).every(([key, value]) => {
    const n = isLosslessNumber(value) ? Number(value.value) : value
    return (
      typeof n === 'number' &&
      Number.isFinite(n) &&
      ((key === 'temperature' && n >= 0 && n <= 2) ||
        (key === 'top_p' && n > 0 && n <= 1))
    )
  })
  const validHeaders = Object.entries(headers).every(
    ([key, value]) =>
      (key === 'user-agent' || key === 'x-client-version') &&
      typeof value === 'string' &&
      value.length >= 1 &&
      value.length <= 256 &&
      !/[^ -~]/.test(value)
  )
  const validStatus = Object.entries(statusCodes).every(
    ([key, value]) =>
      /^[45]\d{2}$/.test(key) &&
      typeof value === 'number' &&
      Number.isInteger(value) &&
      value >= 400 &&
      value <= 599
  )
  if (!validParameters || !validHeaders || !validStatus) {
    throw new Error('Invalid configuration')
  }
  return {
    parameters,
    headers: headers as Record<string, string>,
    status_codes: statusCodes as Record<string, number>,
  }
}

function OverrideForm({
  policy,
  preview,
  busy,
  id,
}: PolicyFormProps<ChannelRules> & { id: number }) {
  const { t } = useTranslation()
  const current = policy[id] ?? empty
  // Keep fractional tokens on the same exact JSON path as the routing document.
  const [parameters, setParameters] = useState(() =>
    stringifyNativeDocument(current.parameters, 2, [
      'channel_overrides',
      String(id),
      'parameters',
    ])
  )
  const [headers, setHeaders] = useState(() =>
    stringifyNativeDocument(current.headers, 2)
  )
  const [codes, setCodes] = useState(() =>
    stringifyNativeDocument(current.status_codes, 2)
  )
  const [error, setError] = useState(false)
  const fields = [
    {
      label: 'Parameters',
      hint: 'temperature · top_p',
      value: parameters,
      onChange: setParameters,
    },
    {
      label: 'Header Override',
      hint: 'user-agent · x-client-version',
      value: headers,
      onChange: setHeaders,
    },
    {
      label: 'Status Code Mapping',
      hint: '400–599 → 400–599',
      value: codes,
      onChange: setCodes,
    },
  ]
  return (
    <form
      className='space-y-4'
      onSubmit={async (event) => {
        event.preventDefault()
        try {
          const next = { ...policy }
          const override = checkedOverrides(id, parameters, headers, codes)
          if (Object.values(override).some((map) => Object.keys(map).length)) {
            next[id] = override
          } else delete next[id]
          setError(false)
          await preview(next)
        } catch {
          setError(true)
        }
      }}
    >
      {fields.map(({ label, hint, value, onChange }) => (
        <div className='space-y-1' key={label}>
          <Label htmlFor={`channel-override-${id}-${label}`}>{t(label)}</Label>
          <p className='text-muted-foreground text-xs'>{hint}</p>
          <Textarea
            id={`channel-override-${id}-${label}`}
            rows={3}
            spellCheck={false}
            className='font-mono text-xs'
            value={value}
            onChange={(e) => onChange(e.target.value)}
          />
        </div>
      ))}
      {error && (
        <p role='alert'>
          {t('Invalid JSON format or values out of allowed range')}
        </p>
      )}
      <Button type='submit' disabled={busy}>
        {t('Preview changes')}
      </Button>
    </form>
  )
}

export function NativeChannelOverridesDialog({
  id,
  onClose,
  onPublished,
}: {
  id: number
  onClose: () => void
  onPublished: () => void
}) {
  const { t } = useTranslation()
  const Editor = useMemo(
    () => (props: PolicyFormProps<ChannelRules>) => (
      <OverrideForm id={id} {...props} />
    ),
    [id]
  )
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={`${t('Advanced Settings')} · #${id}`}
    >
      <NativePolicySection
        name='channel_overrides'
        draft={{} as ChannelRules}
        editor={Editor}
        project={(value) => ({
          channel_overrides: { [id]: value[id] ?? null },
        })}
        onPublished={onPublished}
      />
    </Dialog>
  )
}
