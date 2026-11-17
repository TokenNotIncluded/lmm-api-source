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
import {
  Alert02Icon,
  CheckmarkCircle02Icon,
  InformationCircleIcon,
  Loading03Icon,
  Plug01Icon,
} from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import * as z from 'zod'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
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

import { formatHeroSmsPlatformAmount } from '../../email-activations/api.js'
import { useHeroSmsTranslations } from '../../email-activations/use-hero-sms-translations.js'
import {
  SettingsForm,
  SettingsFormGrid,
  SettingsFormGridItem,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import {
  SettingsPageActionsPortal,
  SettingsPageFormActions,
  SettingsPageTitleStatusPortal,
} from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useResetForm } from '../hooks/use-reset-form'
import {
  getHeroSmsPreviewCustomerPrice,
  getHeroSmsSettings,
  parseHeroSmsSettingsError,
  testHeroSmsConnection,
  toHeroSmsSettingsFormValues,
  updateHeroSmsSettings,
} from './hero-sms-api'

const heroSmsSettingsSchema = z.object({
  revision: z.string().regex(/^(0|[1-9][0-9]*)$/),
  enabled: z.boolean(),
  emailEnabled: z.boolean(),
  smsEnabled: z.boolean(),
  priceMultiplier: z
    .number()
    .min(0.000001)
    .max(1000)
    .refine((value) => /^\d+(?:\.\d{1,6})?$/.test(String(value)), {
      message: 'Use at most 6 decimal places',
    }),
})

type HeroSmsSettingsFormValues = z.infer<typeof heroSmsSettingsSchema>

export function HeroSmsSettingsSection() {
  const { t } = useTranslation()
  useHeroSmsTranslations()
  const queryClient = useQueryClient()
  const [saveError, setSaveError] = useState<string | null>(null)
  const [testState, setTestState] = useState<{
    loading: boolean
    ok: boolean | null
    error: string | null
  }>({ loading: false, ok: null, error: null })

  const settingsQuery = useQuery({
    queryKey: ['hero-sms-settings'],
    queryFn: getHeroSmsSettings,
    placeholderData: (previousData) => previousData,
  })

  const updateMutation = useMutation({
    mutationFn: updateHeroSmsSettings,
    retry: false,
    onMutate: () => setSaveError(null),
    onError: (error) => {
      const { code } = parseHeroSmsSettingsError(error)
      const message =
        code === 'STATE_CONFLICT'
          ? t(
              'HeroSMS settings changed. Reload the latest settings before saving again.'
            )
          : t('Unable to save HeroSMS settings')
      setSaveError(message)
      toast.error(message)
    },
    onSuccess: (settings) => {
      queryClient.setQueryData(['hero-sms-settings'], settings)
      toast.success(t('HeroSMS settings saved'))
    },
  })

  const formDefaults = useMemo(
    () =>
      settingsQuery.data
        ? toHeroSmsSettingsFormValues(settingsQuery.data)
        : undefined,
    [settingsQuery.data]
  )

  const form = useForm<HeroSmsSettingsFormValues>({
    resolver: zodResolver(heroSmsSettingsSchema),
    defaultValues: {
      revision: '0',
      enabled: false,
      emailEnabled: false,
      smsEnabled: false,
      priceMultiplier: 10,
    },
  })
  const priceMultiplier = useWatch({
    control: form.control,
    name: 'priceMultiplier',
  })
  const serviceEnabled = useWatch({ control: form.control, name: 'enabled' })

  useResetForm(form, formDefaults)

  const configured = settingsQuery.data?.api_key_configured ?? false
  const pendingWork = settingsQuery.data?.pending_work ?? false
  const purchasesAvailable = settingsQuery.data?.purchases_available ?? false
  const enabled = serviceEnabled
  const handleSubmit = async (values: HeroSmsSettingsFormValues) => {
    if (values.enabled && !configured) {
      setSaveError(
        t(
          'Configure the HeroSMS credential on the host before enabling purchases.'
        )
      )
      return
    }
    try {
      const settings = await updateMutation.mutateAsync(values)
      form.reset(toHeroSmsSettingsFormValues(settings))
    } catch {
      // The mutation's onError handler owns the translated inline feedback.
    }
  }

  const handleTestConnection = async () => {
    setTestState({ loading: true, ok: null, error: null })
    try {
      await testHeroSmsConnection()
      setTestState({ loading: false, ok: true, error: null })
      toast.success(t('HeroSMS connection test passed'))
    } catch {
      const message = t('Connection failed')
      setTestState({ loading: false, ok: false, error: message })
      toast.error(message)
    }
  }

  const resetToServerState = () => {
    if (formDefaults) form.reset(formDefaults)
    setTestState({ loading: false, ok: null, error: null })
  }

  const reloadSettings = async () => {
    const result = await settingsQuery.refetch()
    if (result.data && !result.isError) {
      form.reset(toHeroSmsSettingsFormValues(result.data))
      setSaveError(null)
      setTestState({ loading: false, ok: null, error: null })
    }
  }

  if (settingsQuery.isLoading && !settingsQuery.data) {
    return <LoadingState message={t('Loading HeroSMS settings...')} />
  }

  if (settingsQuery.isError && !settingsQuery.data) {
    return (
      <ErrorState
        title={t('Unable to load HeroSMS settings')}
        description={t(
          'Retry to fetch the latest provider configuration before editing this section.'
        )}
        onRetry={() => void settingsQuery.refetch()}
      />
    )
  }

  return (
    <SettingsSection title={t('HeroSMS temporary activations')}>
      <SettingsPageTitleStatusPortal>
        <div className='flex flex-wrap items-center gap-2'>
          <Badge variant={configured ? 'default' : 'outline'}>
            <HugeiconsIcon
              icon={configured ? CheckmarkCircle02Icon : InformationCircleIcon}
              data-icon='inline-start'
              strokeWidth={2}
            />
            <span>{configured ? t('Configured') : t('Not configured')}</span>
          </Badge>
          <Badge variant={enabled ? 'secondary' : 'outline'}>
            <span>{enabled ? t('Enabled') : t('Disabled')}</span>
          </Badge>
        </div>
      </SettingsPageTitleStatusPortal>

      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(handleSubmit)}>
          <SettingsPageActionsPortal>
            <Button
              type='button'
              size='sm'
              variant='outline'
              onClick={() => void handleTestConnection()}
              disabled={
                !configured ||
                testState.loading ||
                settingsQuery.isLoading ||
                updateMutation.isPending
              }
            >
              <HugeiconsIcon
                icon={testState.loading ? Loading03Icon : Plug01Icon}
                data-icon='inline-start'
                className={testState.loading ? 'animate-spin' : undefined}
                strokeWidth={2}
              />
              <span>{t('Test connection')}</span>
            </Button>
          </SettingsPageActionsPortal>

          <SettingsPageFormActions
            onSave={form.handleSubmit(handleSubmit)}
            onReset={resetToServerState}
            isSaving={updateMutation.isPending}
            isSaveDisabled={!form.formState.isDirty}
            isResetDisabled={!form.formState.isDirty}
          />

          <Alert>
            <HugeiconsIcon
              icon={InformationCircleIcon}
              strokeWidth={2}
              aria-hidden='true'
            />
            <AlertTitle>
              {t('HeroSMS API key is managed on the host')}
            </AlertTitle>
            <AlertDescription>
              {t(
                'Configure or rotate the credential file on the host. You can save activation options and the price multiplier here, and test the configured credential.'
              )}
            </AlertDescription>
          </Alert>

          {!purchasesAvailable ? (
            <Alert>
              <HugeiconsIcon
                icon={InformationCircleIcon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('Purchases unavailable')}</AlertTitle>
              <AlertDescription>
                {t(
                  'You can save settings and test the connection. SMS and email purchases are not available on this server.'
                )}
              </AlertDescription>
            </Alert>
          ) : null}

          {pendingWork ? (
            <Alert>
              <HugeiconsIcon
                icon={InformationCircleIcon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>
                {t('Active orders are still being reconciled')}
              </AlertTitle>
              <AlertDescription>
                {t(
                  'Keep the HeroSMS API key until active orders finish or are refunded.'
                )}
              </AlertDescription>
            </Alert>
          ) : null}

          {saveError ? (
            <Alert variant='destructive'>
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('Unable to save HeroSMS settings')}</AlertTitle>
              <AlertDescription>
                <p>{saveError}</p>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  disabled={settingsQuery.isFetching}
                  onClick={() => void reloadSettings()}
                >
                  {t('Reload')}
                </Button>
              </AlertDescription>
            </Alert>
          ) : null}

          {settingsQuery.isError && settingsQuery.data ? (
            <Alert variant='destructive'>
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('Using last loaded HeroSMS settings')}</AlertTitle>
              <AlertDescription>
                {t(
                  'Saving is still available, but refresh again if you suspect the server state changed elsewhere.'
                )}
              </AlertDescription>
            </Alert>
          ) : null}

          {!enabled && purchasesAvailable ? (
            <Alert>
              <HugeiconsIcon
                icon={InformationCircleIcon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('HeroSMS purchasing is disabled')}</AlertTitle>
              <AlertDescription>
                {t(
                  'Turn this on only after the API key, multiplier, and test connection all succeed.'
                )}
              </AlertDescription>
            </Alert>
          ) : null}

          {testState.ok ? (
            <Alert>
              <HugeiconsIcon
                icon={CheckmarkCircle02Icon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('Connection test succeeded')}</AlertTitle>
              <AlertDescription>
                {t(
                  'The server can reach HeroSMS with the configured host credential.'
                )}
              </AlertDescription>
            </Alert>
          ) : null}

          {testState.error ? (
            <Alert variant='destructive'>
              <HugeiconsIcon
                icon={Alert02Icon}
                strokeWidth={2}
                aria-hidden='true'
              />
              <AlertTitle>{t('Connection test failed')}</AlertTitle>
              <AlertDescription>{testState.error}</AlertDescription>
            </Alert>
          ) : null}

          <FormField
            control={form.control}
            name='enabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable temporary activations')}</FormLabel>
                  {purchasesAvailable ? (
                    <FormDescription>
                      {t(
                        'Allow authenticated users to purchase temporary phone-number and email activations.'
                      )}
                    </FormDescription>
                  ) : null}
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                  />
                </FormControl>
              </SettingsSwitchItem>
            )}
          />

          <FormField
            control={form.control}
            name='smsEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable phone-number activations')}</FormLabel>
                  {purchasesAvailable ? (
                    <FormDescription>
                      {t(
                        'Allow users to purchase temporary phone numbers and receive SMS verification codes.'
                      )}
                    </FormDescription>
                  ) : null}
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                    disabled={!serviceEnabled}
                  />
                </FormControl>
              </SettingsSwitchItem>
            )}
          />

          <FormField
            control={form.control}
            name='emailEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable email activations')}</FormLabel>
                  {purchasesAvailable ? (
                    <FormDescription>
                      {t(
                        'Allow users to purchase temporary email addresses and receive verification messages.'
                      )}
                    </FormDescription>
                  ) : null}
                </SettingsSwitchContent>
                <FormControl>
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                    disabled={!serviceEnabled}
                  />
                </FormControl>
              </SettingsSwitchItem>
            )}
          />

          <SettingsFormGrid>
            <SettingsFormGridItem>
              <FormField
                control={form.control}
                name='priceMultiplier'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Price multiplier')}</FormLabel>
                    <FormControl>
                      <Input
                        value={String(field.value ?? '')}
                        onChange={(event) => {
                          const nextValue = Number(event.target.value)
                          field.onChange(
                            Number.isFinite(nextValue) ? nextValue : 0
                          )
                        }}
                        type='number'
                        min={0.000001}
                        max={1000}
                        step='0.000001'
                        inputMode='decimal'
                      />
                    </FormControl>
                    <FormDescription>
                      {t('1 USD provider cost → {{price}} platform price', {
                        price: formatHeroSmsPlatformAmount(
                          getHeroSmsPreviewCustomerPrice(priceMultiplier || 1)
                        ),
                      })}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </SettingsFormGridItem>

            <SettingsFormGridItem className='md:col-span-2'>
              <FormItem>
                <FormLabel>{t('Charging rule')}</FormLabel>
                <FormControl>
                  <Input
                    value={t('1 USD provider cost → {{price}} platform price', {
                      price: formatHeroSmsPlatformAmount(
                        getHeroSmsPreviewCustomerPrice(priceMultiplier || 1)
                      ),
                    })}
                    readOnly
                    aria-readonly='true'
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'The multiplier is applied to provider cost to calculate platform price. Provider money and platform credit are distinct units; verify the preview before saving.'
                  )}
                </FormDescription>
              </FormItem>
            </SettingsFormGridItem>
          </SettingsFormGrid>
        </SettingsForm>
      </Form>
    </SettingsSection>
  )
}
