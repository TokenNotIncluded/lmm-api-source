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
import { useMemo } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import * as z from 'zod'

import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { federatedProviders } from '@/features/auth/lib/federated-oauth'
import { useStatus } from '@/hooks/use-status'

import {
  SettingsForm,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useResetForm } from '../hooks/use-reset-form'
import { useUpdateOption } from '../hooks/use-update-option'

const basicAuthSchema = z.object({
  PasswordLoginEnabled: z.boolean(),
  PasswordRegisterEnabled: z.boolean(),
  OAuthRegisterEnabled: z.boolean(),
  RegistrationDisabledMethods: z.string(),
  EmailVerificationEnabled: z.boolean(),
  RegisterEnabled: z.boolean(),
  EmailDomainRestrictionEnabled: z.boolean(),
  EmailAliasRestrictionEnabled: z.boolean(),
  EmailDomainWhitelist: z.string(),
})

type BasicAuthFormValues = z.infer<typeof basicAuthSchema>

type BasicAuthSectionProps = {
  defaultValues: BasicAuthFormValues
}

export function BasicAuthSection({ defaultValues }: BasicAuthSectionProps) {
  const { t } = useTranslation()
  const updateOption = useUpdateOption()
  const { status } = useStatus()

  const formDefaults = useMemo<BasicAuthFormValues>(
    () => ({
      ...defaultValues,
      RegistrationDisabledMethods:
        defaultValues.RegistrationDisabledMethods.split(/[\n,\r]/)
          .map((method) => method.trim().toLowerCase())
          .filter(Boolean)
          .sort((left, right) => left.localeCompare(right))
          .join(','),
      EmailDomainWhitelist: defaultValues.EmailDomainWhitelist.split(',')
        .map((domain) => domain.trim())
        .filter(Boolean)
        .join('\n'),
    }),
    [defaultValues]
  )

  const form = useForm<BasicAuthFormValues>({
    resolver: zodResolver(basicAuthSchema),
    defaultValues: formDefaults,
  })

  useResetForm(form, formDefaults)

  const [registrationEnabled, oauthRegistrationEnabled] = useWatch({
    control: form.control,
    name: ['RegisterEnabled', 'OAuthRegisterEnabled'],
  })
  const oauthRegistrationAllowed =
    registrationEnabled && oauthRegistrationEnabled

  const onSubmit = async (data: BasicAuthFormValues) => {
    const updates: Array<{ key: string; value: string | boolean }> = []

    Object.entries(data).forEach(([key, value]) => {
      if (key === 'EmailDomainWhitelist') {
        if (typeof value !== 'string') return
        const domains = value
          .split('\n')
          .map((domain) => domain.trim())
          .filter(Boolean)
          .join(',')
        if (domains !== defaultValues.EmailDomainWhitelist) {
          updates.push({ key, value: domains })
        }
      } else if (key === 'RegistrationDisabledMethods') {
        if (typeof value !== 'string') return
        const methods = value
          .split(/[\n,\r]/)
          .map((method) => method.trim().toLowerCase())
          .filter(Boolean)
          .sort((left, right) => left.localeCompare(right))
          .join(',')
        if (methods !== defaultValues.RegistrationDisabledMethods) {
          updates.push({ key, value: methods })
        }
      } else if (value !== defaultValues[key as keyof typeof defaultValues]) {
        updates.push({ key, value })
      }
    })

    for (const update of updates) {
      await updateOption.mutateAsync(update)
    }
  }

  return (
    <SettingsSection title={t('Basic Authentication')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaving={updateOption.isPending}
          />
          <FormField
            control={form.control}
            name='PasswordLoginEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Password Login')}</FormLabel>
                  <FormDescription>
                    {t('Allow users to log in with password')}
                  </FormDescription>
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
            name='RegisterEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Registration Enabled')}</FormLabel>
                  <FormDescription>
                    {t('Allow new users to register')}
                  </FormDescription>
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
            name='PasswordRegisterEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Password Registration')}</FormLabel>
                  <FormDescription>
                    {t('Allow registration with password')}
                  </FormDescription>
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
            name='OAuthRegisterEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('OAuth Registration')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Allow new accounts through OAuth providers. Existing users can still sign in.'
                    )}
                  </FormDescription>
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
            name='RegistrationDisabledMethods'
            render={({ field }) => {
              const disabledMethods = new Set(
                field.value
                  .split(/[\n,\r]/)
                  .map((method) => method.trim().toLowerCase())
                  .filter(Boolean)
              )
              const methods = [
                {
                  id: 'google',
                  label: 'Google',
                  parentIds: [] as string[],
                  aliases: [] as string[],
                },
                {
                  id: 'github',
                  label: t('GitHub'),
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'discord',
                  label: t('Discord'),
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'oidc',
                  label: 'OIDC',
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'custom_oauth',
                  label: t('Custom OAuth'),
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'linuxdo',
                  label: t('LinuxDO'),
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'telegram',
                  label: t('Telegram'),
                  parentIds: [],
                  aliases: [],
                },
                {
                  id: 'wechat',
                  label: t('WeChat'),
                  parentIds: [],
                  aliases: [],
                },
                ...federatedProviders(status).map((provider) => ({
                  id: `federated:${provider.id}`,
                  label: provider.name,
                  parentIds: [provider.kind],
                  aliases:
                    provider.kind === 'custom_oauth'
                      ? [`custom:${provider.slug}`]
                      : [],
                })),
              ]

              return (
                <FormItem className='lg:col-span-2'>
                  <FormLabel>{t('Registration channels')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Disable selected OAuth channels for new registrations only. Existing users can still sign in with them.'
                    )}
                  </FormDescription>
                  <div className='grid gap-2 lg:grid-cols-2'>
                    {methods.map((method) => {
                      const loginOnly =
                        method.id === 'telegram' &&
                        status?.telegram_register_enabled !== true
                      const inheritedBlock = method.parentIds.some((id) =>
                        disabledMethods.has(id)
                      )
                      const allowed =
                        oauthRegistrationAllowed &&
                        !loginOnly &&
                        !inheritedBlock &&
                        ![method.id, ...method.aliases].some((id) =>
                          disabledMethods.has(id)
                        )
                      return (
                        <SettingsSwitchItem key={method.id}>
                          <SettingsSwitchContent>
                            <FormLabel>{method.label}</FormLabel>
                            <FormDescription>
                              {allowed
                                ? t('Sign in or register')
                                : t('Sign in only')}
                              {inheritedBlock ? (
                                <span className='mt-1 block'>
                                  {t(
                                    'Registration is disabled by a channel-wide rule.'
                                  )}
                                </span>
                              ) : null}
                            </FormDescription>
                          </SettingsSwitchContent>
                          <FormControl>
                            <Switch
                              aria-label={t(
                                'Allow new accounts through {{method}}',
                                {
                                  method: method.label,
                                }
                              )}
                              checked={allowed}
                              disabled={
                                !oauthRegistrationAllowed ||
                                inheritedBlock ||
                                loginOnly
                              }
                              onCheckedChange={(allowed) => {
                                const next = new Set(disabledMethods)
                                if (allowed) {
                                  for (const id of [
                                    method.id,
                                    ...method.aliases,
                                  ]) {
                                    next.delete(id)
                                  }
                                } else next.add(method.id)
                                field.onChange(
                                  [...next]
                                    .sort((left, right) =>
                                      left.localeCompare(right)
                                    )
                                    .join(',')
                                )
                              }}
                            />
                          </FormControl>
                        </SettingsSwitchItem>
                      )
                    })}
                  </div>
                  <FormMessage />
                </FormItem>
              )
            }}
          />

          <FormField
            control={form.control}
            name='EmailVerificationEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Email Verification')}</FormLabel>
                  <FormDescription>
                    {t('Require email verification for new accounts')}
                  </FormDescription>
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
            name='EmailDomainRestrictionEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Email Domain Restriction')}</FormLabel>
                  <FormDescription>
                    {t('Only allow specific email domains')}
                  </FormDescription>
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
            name='EmailAliasRestrictionEnabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Email Alias Restriction')}</FormLabel>
                  <FormDescription>
                    {t('Block email aliases (e.g., user+alias@domain.com)')}
                  </FormDescription>
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
            name='EmailDomainWhitelist'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Email Domain Whitelist')}</FormLabel>
                <FormControl>
                  <Textarea
                    placeholder={t('example.com&#10;company.com')}
                    rows={4}
                    {...field}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'One domain per line (only used when domain restriction is enabled)'
                  )}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
        </SettingsForm>
      </Form>
    </SettingsSection>
  )
}
