/*
Copyright (C) 2026 LIghtJUNction
*/
import { zodResolver } from '@hookform/resolvers/zod'
import { useMemo, useState } from 'react'
import type { Resolver } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Form } from '@/components/ui/form'
import { useAuthStore } from '@/stores/auth-store'

import { getSystemOptions, updateSystemOption } from '../api'
import { FormDirtyIndicator } from '../components/form-dirty-indicator'
import { FormNavigationGuard } from '../components/form-navigation-guard'
import { SettingsForm } from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useSettingsForm } from '../hooks/use-settings-form'
import { useSystemOptions } from '../hooks/use-system-options'
import { getSettingsErrorMessage } from '../utils/settings-error-message'
import { TrustLevelBenefitsFields } from './trust-level-benefits-fields'
import {
  parseTrustLevelBenefits,
  serializeTrustLevelBenefits,
  TRUST_LEVEL_BENEFITS_OPTION,
  trustLevelBenefitsSchema,
  type TrustLevelBenefitsConfig,
} from './trust-level-benefits-schema'
import { TrustPolicyEditor } from './trust-policy-editor'

export function TrustLevelBenefitsSection({ value }: { value?: string }) {
  const { t } = useTranslation()
  const { data } = useSystemOptions()
  const raw =
    value ??
    data?.data.find((option) => option.key === TRUST_LEVEL_BENEFITS_OPTION)
      ?.value ??
    ''
  const config = useMemo(() => parseTrustLevelBenefits(raw), [raw])
  const supported = data?.capabilities?.trust_level_benefits === true
  return (
    <SettingsSection title={t('Levels & Benefits')}>
      {data?.capabilities?.partial === true ? (
        <TrustPolicyEditor enabled={supported} />
      ) : config && supported ? (
        <TrustLevelBenefitsForm defaultValues={config} />
      ) : (
        <Alert variant='destructive'>
          <AlertTitle>{t('Level configuration unavailable')}</AlertTitle>
          <AlertDescription>
            {t(
              supported
                ? 'Load a valid level configuration from the server before editing. No recharge thresholds have been assumed.'
                : 'The server does not support configurable level benefits. Update the server before editing.'
            )}
          </AlertDescription>
        </Alert>
      )}
    </SettingsSection>
  )
}

function TrustLevelBenefitsForm({
  defaultValues,
}: {
  defaultValues: TrustLevelBenefitsConfig
}) {
  const { t } = useTranslation()
  const [saveError, setSaveError] = useState<string | null>(null)
  const { form, handleSubmit, handleReset, isDirty, isSubmitting } =
    useSettingsForm<TrustLevelBenefitsConfig>({
      defaultValues,
      resolver: zodResolver(
        trustLevelBenefitsSchema
      ) as Resolver<TrustLevelBenefitsConfig>,
      onSubmit: async (data) => {
        setSaveError(null)
        try {
          const auth = useAuthStore.getState().auth
          const authScope = {
            userId: auth.user?.id,
            sessionId: auth.session?.sid,
          }
          const current = await getSystemOptions({ silent: true, authScope })
          const live = useAuthStore.getState().auth
          if (
            live.user?.id !== authScope.userId ||
            live.session?.sid !== authScope.sessionId
          ) {
            throw new Error(t('Read only'))
          }
          if (
            !current.success ||
            current.capabilities?.trust_level_benefits !== true ||
            current.capabilities.partial === true ||
            current.capabilities.read_only === true ||
            !current.capabilities.writable_keys?.includes(
              TRUST_LEVEL_BENEFITS_OPTION
            )
          ) {
            throw new Error(
              current.message ||
                t(
                  'The server does not support configurable level benefits. Update the server before editing.'
                )
            )
          }
          // Dedicated legacy compatibility: the generic option allowlist stays unchanged.
          // Only the live backend declaration can authorize this exact PUT key.
          const result = await updateSystemOption(
            {
              key: TRUST_LEVEL_BENEFITS_OPTION,
              value: serializeTrustLevelBenefits(data),
            },
            { silent: true, authScope }
          )
          if (result.success !== true) {
            throw new Error(result.message || t('Failed to update setting'))
          }
        } catch (error) {
          setSaveError(
            getSettingsErrorMessage(error, t('Failed to update setting'))
          )
          throw error
        }
      },
    })
  const saving = isSubmitting
  const submit = () => {
    void handleSubmit().catch(() => {
      /* The mutation reports the error and the draft stays dirty. */
    })
  }
  return (
    <>
      <FormNavigationGuard when={isDirty} />
      <Alert>
        <AlertTitle>{t('Automatic recharge levels (L0–L4)')}</AlertTitle>
        <AlertDescription>
          {t(
            'Set four cumulative recharge thresholds for L1–L4. These levels never grant administrator roles.'
          )}
        </AlertDescription>
      </Alert>
      <Form {...form}>
        <SettingsForm
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <SettingsPageFormActions
            onSave={submit}
            onReset={handleReset}
            isSaving={saving}
            isSaveDisabled={!isDirty}
            isResetDisabled={!isDirty}
          />
          <FormDirtyIndicator isDirty={isDirty} />
          {saveError && (
            <Alert variant='destructive'>
              <AlertDescription>{saveError}</AlertDescription>
            </Alert>
          )}
          <TrustLevelBenefitsFields
            form={form}
            defaultValues={defaultValues}
            saving={saving}
          />
        </SettingsForm>
      </Form>
    </>
  )
}
