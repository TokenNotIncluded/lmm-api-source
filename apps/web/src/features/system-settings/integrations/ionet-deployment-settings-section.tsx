/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { zodResolver } from '@hookform/resolvers/zod'
import { CheckCircle2, Loader2, XCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { z } from 'zod'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { testDeploymentConnectionWithKey } from '@/features/models/api'
import { clearConnectionCache } from '@/features/models/hooks/use-model-deployment-settings'
import {
  readIoNetSettings,
  saveIoNetSettings,
} from '@/features/models/ionet-api'
import { openExternalUrl } from '@/lib/external-navigation'
import { ROLE } from '@/lib/roles'
import { validatedExternalUrl } from '@/lib/validated-external-url'
import { useAuthStore } from '@/stores/auth-store'

import {
  SettingsForm,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'

const schema = z.object({
  enabled: z.boolean(),
  apiKey: z.string().optional(),
})

// NOTE: react-hook-form resolver uses the schema input type
type Values = z.input<typeof schema>

export function IoNetDeploymentSettingsSection(_props: {
  defaultValues: { enabled: boolean; apiKey: string }
}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const canManage = auth.user?.role === ROLE.SUPER_ADMIN
  const [revision, setRevision] = useState<number | null>(null)
  const [configured, setConfigured] = useState(false)
  const [clearKey, setClearKey] = useState(false)
  const [saving, setSaving] = useState(false)
  const form = useForm<Values>({
    resolver: zodResolver(schema),
    defaultValues: {
      enabled: false,
      apiKey: '',
    },
  })

  const { isDirty, isSubmitting } = form.formState
  const enabled = form.watch('enabled')

  const [testState, setTestState] = useState<{
    loading: boolean
    ok: boolean | null
    error: string | null
  }>({ loading: false, ok: null, error: null })

  useEffect(() => {
    const controller = new AbortController()
    setRevision(null)
    setClearKey(false)
    setConfigured(false)
    form.reset({ enabled: false, apiKey: '' })
    void readIoNetSettings(controller.signal)
      .then((settings) => {
        if (controller.signal.aborted) return
        setRevision(settings.revision)
        setConfigured(settings.configured)
        form.reset({ enabled: settings.enabled, apiKey: '' })
      })
      .catch(() => {
        if (!controller.signal.aborted)
          toast.error(t('Failed to load settings'))
      })
    return () => controller.abort()
  }, [auth.user?.id, auth.user?.role, auth.session?.sid, form, t])
  async function onSubmit(values: Values) {
    if (!canManage || revision === null) return
    setSaving(true)
    try {
      const settings = await saveIoNetSettings({
        revision,
        enabled: values.enabled,
        api_key: values.apiKey?.trim() || '',
        clear_api_key: clearKey,
      })
      setRevision(settings.revision)
      setConfigured(settings.configured)
      setClearKey(false)
      form.reset({ enabled: settings.enabled, apiKey: '' })
      clearConnectionCache()
      toast.success(t('Saved successfully'))
    } catch {
      toast.error(t('Failed to save'))
    } finally {
      setSaving(false)
    }
  }

  const handleTestConnection = async () => {
    setTestState({ loading: true, ok: null, error: null })
    try {
      const apiKey = form.getValues('apiKey')
      const res = await testDeploymentConnectionWithKey(apiKey)
      if (res?.success) {
        setTestState({ loading: false, ok: true, error: null })
        return
      }
      setTestState({
        loading: false,
        ok: false,
        error: res?.message || t('Connection failed'),
      })
    } catch (err) {
      setTestState({
        loading: false,
        ok: false,
        error: err instanceof Error ? err.message : t('Connection failed'),
      })
    }
  }

  return (
    <SettingsSection title={t('io.net Deployments')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)} autoComplete='off'>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={saving || isSubmitting}
            isSaveDisabled={
              !canManage || revision === null || (!isDirty && !clearKey)
            }
            saveLabel='Save io.net settings'
          />
          <FormField
            control={form.control}
            name='enabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable io.net deployments')}</FormLabel>
                  <FormDescription>
                    {t('Enable io.net model deployment service in console')}
                  </FormDescription>
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={(v) => field.onChange(v)}
                    disabled={
                      !canManage || revision === null || saving || isSubmitting
                    }
                  />
                </FormControl>
              </SettingsSwitchItem>
            )}
          />

          {configured ? (
            <label className='flex items-center gap-2 text-sm'>
              <input
                type='checkbox'
                checked={clearKey}
                disabled={!canManage || revision === null || saving}
                onChange={(event) => {
                  setClearKey(event.target.checked)
                  if (event.target.checked) {
                    form.setValue('apiKey', '', { shouldDirty: true })
                    form.setValue('enabled', false, { shouldDirty: true })
                  }
                }}
              />
              {t('Clear')} {t('io.net API Key')}
            </label>
          ) : null}
          {enabled ? (
            <>
              <FormField
                control={form.control}
                name='apiKey'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('io.net API Key')}</FormLabel>
                    <div className='flex gap-2'>
                      <FormControl>
                        <Input
                          type='password'
                          placeholder={
                            configured
                              ? t('Key is configured; leave blank to keep it')
                              : t('Enter API Key')
                          }
                          disabled={!canManage || revision === null || saving}
                          onInput={() => setClearKey(false)}
                          autoComplete='off'
                          {...field}
                        />
                      </FormControl>
                      <Button
                        type='button'
                        variant='secondary'
                        onClick={handleTestConnection}
                        disabled={testState.loading || saving}
                        className='shrink-0'
                      >
                        {testState.loading ? (
                          <Loader2 className='me-2 size-4 animate-spin' />
                        ) : null}
                        {t('Test Connection')}
                      </Button>
                    </div>
                    <FormDescription>
                      {t('Used to authenticate with io.net deployment API')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <Alert variant='default'>
                <AlertTitle>{t('How to get an io.net API Key')}</AlertTitle>
                <AlertDescription>
                  <div className='space-y-2'>
                    <ul className='list-disc space-y-1 pl-5'>
                      <li>{t('Open the io.net console API Keys page')}</li>
                      <li>
                        {t(
                          'Set Project to io.cloud when creating/selecting key'
                        )}
                      </li>
                      <li>{t('Copy the key and paste it here')}</li>
                    </ul>
                    <Button
                      type='button'
                      variant='outline'
                      onClick={async () => {
                        const documentationUrl = validatedExternalUrl(
                          'https://ai.io.net/ai/api-keys',
                          {
                            protocols: ['https:'],
                            origins: ['https://ai.io.net'],
                            hosts: ['ai.io.net'],
                            paths: { exact: ['/ai/api-keys'] },
                          }
                        )
                        if (documentationUrl) {
                          // Invariant: documentationUrl is the exact HTTPS ai.io.net API-key page.
                          // pi-lens-ignore: ts-open-redirect, no-open-redirect
                          const opened = await openExternalUrl(documentationUrl)
                          if (!opened) toast.error(t('Unable to open link'))
                        }
                      }}
                    >
                      {t('Go to io.net API Keys')}
                    </Button>
                  </div>
                </AlertDescription>
              </Alert>

              {testState.ok === true ? (
                <Alert variant='default' className='flex items-center gap-2'>
                  <CheckCircle2 className='console-status-success size-4' />
                  <div>
                    <AlertTitle>{t('Connection successful')}</AlertTitle>
                    <AlertDescription>
                      {t('Connected to io.net service normally.')}
                    </AlertDescription>
                  </div>
                </Alert>
              ) : null}

              {testState.ok === false && testState.error ? (
                <Alert
                  variant='destructive'
                  className='flex items-center gap-2'
                >
                  <XCircle className='size-4' />
                  <div>
                    <AlertTitle>{t('Connection failed')}</AlertTitle>
                    <AlertDescription>{t(testState.error)}</AlertDescription>
                  </div>
                </Alert>
              ) : null}
            </>
          ) : null}
        </SettingsForm>
      </Form>
    </SettingsSection>
  )
}
