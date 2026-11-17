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
import { useEffect, useRef, useState } from 'react'
import { type Resolver, useForm, useWatch } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

import { CopyButton } from '@/components/copy-button'
import { Dialog } from '@/components/dialog'
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
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { SystemJsonCodeEditor } from '@/features/system-settings/components/system-json-code-editor'

import {
  SettingsForm,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../../../components/settings-form-layout'
import { buildOAuthCallbackUrl } from '../../oauth-callback-url'
import { isProviderWriteResultUnknown } from '../api'
import {
  useCreateProvider,
  useUpdateProvider,
} from '../hooks/use-custom-oauth-mutations'
import {
  customOAuthFormSchema,
  AUTH_STYLE_OPTIONS,
  type CustomOAuthProvider,
  type CustomOAuthFormValues,
} from '../types'
import { DiscoveryButton } from './discovery-button'
import { PresetSelector } from './preset-selector'

type ProviderFormDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  provider?: CustomOAuthProvider | null
  serverAddress: string
}

const PROVIDER_FORM_ID = 'custom-oauth-provider-form'

export function ProviderFormDialog(props: ProviderFormDialogProps) {
  const { t } = useTranslation()
  const isEditing = !!props.provider
  const createProvider = useCreateProvider()
  const updateProvider = useUpdateProvider()
  const isPending = createProvider.isPending || updateProvider.isPending
  const [unknownValues, setUnknownValues] =
    useState<CustomOAuthFormValues | null>(null)

  const form = useForm<CustomOAuthFormValues>({
    // SAFETY: The schema output is CustomOAuthFormValues; the resolver helper
    // loses that generic relationship across the installed Zod versions.
    resolver: zodResolver(
      customOAuthFormSchema
    ) as unknown as Resolver<CustomOAuthFormValues>,
    defaultValues: {
      name: '',
      slug: '',
      icon: '',
      enabled: true,
      client_id: '',
      client_secret: '',
      authorization_endpoint: '',
      token_endpoint: '',
      user_info_endpoint: '',
      scopes: '',
      user_id_field: '',
      username_field: '',
      display_name_field: '',
      email_field: '',
      well_known: '',
      auth_style: 0,
      pkce_s256: false,
      access_policy: '',
      access_denied_message: '',
    },
  })
  const watchedSlug = useWatch({ control: form.control, name: 'slug' })
  const operationKey = useRef('')
  const callbackPath = watchedSlug?.trim() || '{slug}'
  const callbackUrl = buildOAuthCallbackUrl(
    props.serverAddress,
    callbackPath,
    t('Site URL')
  )

  useEffect(() => {
    if (props.open) {
      operationKey.current = crypto.randomUUID()
    }
    if (props.open && props.provider) {
      form.reset({
        name: props.provider.name,
        slug: props.provider.slug,
        icon: props.provider.icon || '',
        enabled: props.provider.enabled,
        client_id: props.provider.client_id,
        client_secret: '',
        authorization_endpoint: props.provider.authorization_endpoint,
        token_endpoint: props.provider.token_endpoint,
        user_info_endpoint: props.provider.user_info_endpoint,
        scopes: props.provider.scopes || '',
        user_id_field: props.provider.user_id_field,
        username_field: props.provider.username_field || '',
        display_name_field: props.provider.display_name_field || '',
        email_field: props.provider.email_field || '',
        well_known: props.provider.well_known || '',
        auth_style: props.provider.auth_style ?? 0,
        pkce_s256: props.provider.pkce_s256,
        access_policy: props.provider.access_policy || '',
        access_denied_message: props.provider.access_denied_message || '',
      })
    } else if (props.open && !props.provider) {
      form.reset({
        name: '',
        slug: '',
        icon: '',
        enabled: true,
        client_id: '',
        client_secret: '',
        authorization_endpoint: '',
        token_endpoint: '',
        user_info_endpoint: '',
        scopes: '',
        user_id_field: '',
        username_field: '',
        display_name_field: '',
        email_field: '',
        well_known: '',
        auth_style: 0,
        pkce_s256: false,
        access_policy: '',
        access_denied_message: '',
      })
    }
  }, [props.open, props.provider, form])

  const onSubmit = async (values: CustomOAuthFormValues) => {
    const submitted = unknownValues ?? { ...values }
    try {
      if (isEditing && props.provider) {
        await updateProvider.mutateAsync({
          provider: props.provider,
          data: submitted,
          operationKey: operationKey.current,
        })
      } else {
        await createProvider.mutateAsync({
          data: submitted,
          operationKey: operationKey.current,
        })
      }
      setUnknownValues(null)
      props.onOpenChange(false)
    } catch (error) {
      // Retain the exact request after a lost response. A changed draft must
      // not be sent with the key of an operation that may already be applied.
      setUnknownValues(isProviderWriteResultUnknown(error) ? submitted : null)
    }
  }

  const handleOpenChange = (open: boolean) => {
    if (isPending) return
    setUnknownValues(null)
    props.onOpenChange(open)
  }

  let submitLabel = t('Create Provider')
  if (isPending) {
    submitLabel = t('Saving...')
  } else if (unknownValues) {
    submitLabel = t('Retry saved request')
  } else if (isEditing) {
    submitLabel = t('Update Provider')
  }

  return (
    <Dialog
      open={props.open}
      onOpenChange={handleOpenChange}
      title={isEditing ? t('Edit OAuth Provider') : t('Add OAuth Provider')}
      description={
        isEditing
          ? t('Update the configuration for this custom OAuth provider.')
          : t('Configure a new custom OAuth provider for user authentication.')
      }
      contentClassName='max-h-[85vh] overflow-y-auto sm:max-w-2xl'
      contentHeight='auto'
      bodyClassName='space-y-4'
      footer={
        <>
          <Button
            type='button'
            variant='outline'
            onClick={() => handleOpenChange(false)}
            disabled={isPending}
          >
            {t('Cancel')}
          </Button>
          <Button
            type={unknownValues ? 'button' : 'submit'}
            form={PROVIDER_FORM_ID}
            disabled={isPending}
            onClick={
              unknownValues ? () => void onSubmit(unknownValues) : undefined
            }
          >
            {submitLabel}
          </Button>
        </>
      }
    >
      {unknownValues && (
        <Alert>
          <AlertDescription>
            {t(
              'Result unknown. Retry the saved request before starting another.'
            )}
          </AlertDescription>
        </Alert>
      )}
      <Form {...form}>
        <SettingsForm
          id={PROVIDER_FORM_ID}
          onSubmit={(event) => void form.handleSubmit(onSubmit)(event)}
        >
          <fieldset
            className='contents'
            disabled={isPending || !!unknownValues}
          >
            {/* Preset Selector (only for creating) */}
            {!isEditing && <PresetSelector form={form} />}

            <Alert>
              <AlertTitle>{t('OAuth callback URL')}</AlertTitle>
              <AlertDescription className='space-y-3 text-sm'>
                <p>
                  {t(
                    'This callback URL updates from the slug field and is the value to register with your provider.'
                  )}
                </p>
                <div className='flex min-w-0 flex-col gap-1.5 sm:flex-row sm:items-center sm:justify-between'>
                  <span className='text-muted-foreground shrink-0'>
                    {t('Authorization callback URL')}
                  </span>
                  <span className='flex min-w-0 items-center gap-2'>
                    <code className='bg-muted text-foreground min-w-0 rounded px-1.5 py-0.5 text-xs break-all'>
                      {callbackUrl}
                    </code>
                    <CopyButton
                      value={callbackUrl}
                      size='icon'
                      className='size-7'
                      tooltip={t('Copy callback URL')}
                      aria-label={t('Copy callback URL')}
                    />
                  </span>
                </div>
              </AlertDescription>
            </Alert>

            {/* Basic Info */}
            <div className='space-y-4'>
              <h4 className='text-sm font-medium'>{t('Basic Info')}</h4>

              <FormField
                control={form.control}
                name='enabled'
                render={({ field }) => (
                  <SettingsSwitchItem>
                    <SettingsSwitchContent>
                      <FormLabel>{t('Enabled')}</FormLabel>
                      <FormDescription>
                        {t('Allow users to sign in with this provider')}
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

              <div className='grid grid-cols-1 gap-4 sm:grid-cols-2'>
                <FormField
                  control={form.control}
                  name='name'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Provider Name')}</FormLabel>
                      <FormControl>
                        <Input placeholder={t('e.g. My GitLab')} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='slug'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Slug')}</FormLabel>
                      <FormControl>
                        <Input placeholder={t('e.g. my-gitlab')} {...field} />
                      </FormControl>
                      <FormDescription>
                        {t('Used in URLs and API routes')}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name='icon'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Icon')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder={t('Icon identifier (e.g. github, gitlab)')}
                        {...field}
                      />
                    </FormControl>
                    <FormDescription>
                      {t('Optional icon identifier for the login button')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>

            <Separator />

            {/* Credentials */}
            <div className='space-y-4'>
              <h4 className='text-sm font-medium'>{t('Credentials')}</h4>
              <div className='grid grid-cols-1 gap-4 sm:grid-cols-2'>
                <FormField
                  control={form.control}
                  name='client_id'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Client ID')}</FormLabel>
                      <FormControl>
                        <Input
                          placeholder={t('OAuth Client ID')}
                          autoComplete='off'
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='client_secret'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Client Secret')}</FormLabel>
                      <FormControl>
                        <Input
                          type='password'
                          placeholder={t('OAuth Client Secret')}
                          autoComplete='new-password'
                          {...field}
                        />
                      </FormControl>
                      {isEditing && props.provider?.secret_configured && (
                        <FormDescription>
                          {t('Leave blank to keep the saved secret')}
                        </FormDescription>
                      )}
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name='auth_style'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Auth Style')}</FormLabel>
                    <Select
                      items={AUTH_STYLE_OPTIONS.map((option) => ({
                        value: String(option.value),
                        label: t(option.labelKey),
                      }))}
                      value={String(field.value)}
                      onValueChange={(val) => field.onChange(Number(val))}
                    >
                      <FormControl>
                        <SelectTrigger className='w-full'>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent alignItemWithTrigger={false}>
                        <SelectGroup>
                          {AUTH_STYLE_OPTIONS.map((option) => (
                            <SelectItem
                              key={option.value}
                              value={String(option.value)}
                            >
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectGroup>
                      </SelectContent>
                    </Select>
                    <FormDescription>
                      {t(
                        'How client credentials are sent to the token endpoint'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='pkce_s256'
                render={({ field }) => (
                  <SettingsSwitchItem>
                    <SettingsSwitchContent>
                      <FormLabel>{t('PKCE (S256)')}</FormLabel>
                      <FormDescription>
                        {t('Use PKCE (S256) if the provider supports it.')}
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
            </div>

            <Separator />

            {/* Endpoints */}
            <div className='space-y-4'>
              <div className='flex items-center justify-between'>
                <h4 className='text-sm font-medium'>{t('Endpoints')}</h4>
                <DiscoveryButton form={form} />
              </div>

              <FormField
                control={form.control}
                name='well_known'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Well-Known URL')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder={t(
                          'https://provider.com/.well-known/openid-configuration'
                        )}
                        {...field}
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'OIDC discovery URL. Click "Auto-discover" to fetch endpoints automatically.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='authorization_endpoint'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Authorization Endpoint')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder='https://provider.com/oauth/authorize'
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='token_endpoint'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Token Endpoint')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder='https://provider.com/oauth/token'
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='user_info_endpoint'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('User Info Endpoint')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder='https://provider.com/api/user'
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='scopes'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Scopes')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder={t('e.g. openid profile email')}
                        {...field}
                      />
                    </FormControl>
                    <FormDescription>
                      {t('Space-separated OAuth scopes')}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>

            <Separator />

            {/* Field Mapping */}
            <div className='space-y-4'>
              <h4 className='text-sm font-medium'>{t('Field Mapping')}</h4>
              <FormDescription>
                {t(
                  'Map fields from the user info response to local user attributes. Supports nested paths (e.g. ocs.data.id).'
                )}
              </FormDescription>

              <div className='grid grid-cols-1 gap-4 sm:grid-cols-2'>
                <FormField
                  control={form.control}
                  name='user_id_field'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('User ID Field')}</FormLabel>
                      <FormControl>
                        <Input placeholder='id' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='username_field'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Username Field')}</FormLabel>
                      <FormControl>
                        <Input placeholder='login' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='display_name_field'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Display Name Field')}</FormLabel>
                      <FormControl>
                        <Input placeholder='name' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name='email_field'
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Email Field')}</FormLabel>
                      <FormControl>
                        <Input placeholder='email' {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
            </div>

            <Separator />

            {/* Advanced */}
            <div className='space-y-4'>
              <h4 className='text-sm font-medium'>{t('Advanced')}</h4>

              <FormField
                control={form.control}
                name='access_policy'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Access Policy (JSON)')}</FormLabel>
                    <FormControl>
                      <SystemJsonCodeEditor
                        configurationKey='custom_oauth.access_policy'
                        value={field.value || ''}
                        onChange={field.onChange}
                        name={field.name}
                        onBlur={field.onBlur}
                        textareaRef={field.ref}
                        placeholder={t(
                          'Optional JSON policy to restrict access based on user info fields'
                        )}
                        heightClassName='h-40 min-h-40 max-h-40'
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        'JSON-based access control rules. Leave empty to allow all users.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name='access_denied_message'
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t('Access Denied Message')}</FormLabel>
                    <FormControl>
                      <Input
                        placeholder={t(
                          'Custom message shown when access is denied'
                        )}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
          </fieldset>
        </SettingsForm>
      </Form>
    </Dialog>
  )
}
