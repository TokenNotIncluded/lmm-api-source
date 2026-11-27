/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { SettingsPageFormActions } from '@/features/system-settings/components/settings-page-context'
import { SettingsSection } from '@/features/system-settings/components/settings-section'
import { getSettingsErrorMessage } from '@/features/system-settings/utils/settings-error-message'

import { MARKET_AI_REVIEW_COPY as copy } from './copy'
import {
  MarketAIReviewModeFields,
  type MarketAIReviewModes,
} from './mode-fields'

export function MarketAIReviewSettingsForm({
  defaultValues,
  market,
  onSave,
}: {
  defaultValues: MarketAIReviewModes
  market?: keyof MarketAIReviewModes
  onSave: (
    updates: Partial<MarketAIReviewModes>
  ) => Promise<MarketAIReviewModes | void>
}) {
  const { t } = useTranslation()
  const baseline = useRef(defaultValues)
  const [draft, setDraft] = useState(defaultValues)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  useEffect(() => {
    const before = baseline.current
    baseline.current = defaultValues
    setDraft((value) => ({
      tool: value.tool === before.tool ? defaultValues.tool : value.tool,
      product:
        value.product === before.product
          ? defaultValues.product
          : value.product,
    }))
  }, [defaultValues])
  const changed =
    (market !== 'product' && draft.tool !== baseline.current.tool) ||
    (market !== 'tool' && draft.product !== baseline.current.product)
  async function save() {
    if (busy || !changed) return
    const submitted = draft
    const updates: Partial<MarketAIReviewModes> = {}
    if (market !== 'product' && submitted.tool !== baseline.current.tool) {
      updates.tool = submitted.tool
    }
    if (market !== 'tool' && submitted.product !== baseline.current.product) {
      updates.product = submitted.product
    }
    setBusy(true)
    setError(null)
    try {
      const confirmed = await onSave(updates)
      const saved = confirmed ?? { ...baseline.current, ...updates }
      baseline.current = saved
      setDraft(saved)
    } catch (issue) {
      setError(issue)
    } finally {
      setBusy(false)
    }
  }
  return (
    <SettingsSection title={t(copy.title)}>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <SettingsPageFormActions
          onSave={() => void save()}
          onReset={() => {
            setDraft(baseline.current)
            setError(null)
          }}
          isSaving={busy}
          isSaveDisabled={!changed}
          isResetDisabled={!changed}
        />
        <MarketAIReviewModeFields
          market={market}
          value={draft}
          onChange={setDraft}
          disabled={busy}
        />
        {Boolean(error) && (
          <p role='alert' className='text-destructive mt-3 text-sm'>
            {getSettingsErrorMessage(error, t('Failed to update setting'))}
          </p>
        )}
      </form>
    </SettingsSection>
  )
}
