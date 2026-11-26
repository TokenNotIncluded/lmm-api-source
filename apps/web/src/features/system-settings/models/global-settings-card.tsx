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
import { useForm, useWatch } from 'react-hook-form'
import { useTranslation } from 'react-i18next'
import * as z from 'zod'

import { StatusBadge } from '@/components/status-badge'
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
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { SystemJsonCodeEditor } from '@/features/system-settings/components/system-json-code-editor'

import {
  SettingsForm,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import { SettingsPageFormActions } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import {
  NativePolicySection,
  type PolicyFormProps,
} from './native-policy-section'

const thinkingBlacklistExample = JSON.stringify(
  ['moonshotai/kimi-k2-thinking', 'kimi-k2-thinking'],
  null,
  2
)

const chatToResponsesPolicyExample = JSON.stringify(
  {
    enabled: true,
    all_channels: false,
    channel_ids: [1, 2],
    model_patterns: ['^gpt-4o.*$', '^gpt-5.*$'],
  },
  null,
  2
)

const chatToResponsesPolicyAllChannelsExample = JSON.stringify(
  {
    enabled: true,
    all_channels: true,
    overrides: { '3': false },
    model_patterns: ['^gpt-4o.*$', '^gpt-5.*$'],
  },
  null,
  2
)

const chatPolicy = z
  .object({
    enabled: z.boolean(),
    all_channels: z.boolean(),
    channel_ids: z.array(z.number().int().positive().safe()).optional(),
    model_patterns: z.array(z.string()),
    overrides: z.record(z.string().regex(/^[1-9]\d*$/), z.boolean()).optional(),
  })
  .strict()
const modelNames = z.array(z.string().min(1))

function jsonField(schema: z.ZodType) {
  return z.string().refine((text) => {
    try {
      return schema.safeParse(JSON.parse(text)).success
    } catch {
      return false
    }
  }, 'Invalid JSON format')
}

const schema = z.object({
  pass_through_request_enabled: z.boolean(),
  thinking_model_blacklist: jsonField(modelNames),
  chat_completions_to_responses_policy: jsonField(chatPolicy),
  ping_interval_enabled: z.boolean(),
  ping_interval_seconds: z.coerce.number().int().min(1).max(3600),
})
type GlobalModelSettingsFormValues = z.output<typeof schema>
type GlobalModelSettingsFormInput = z.input<typeof schema>
type GlobalPolicy = Omit<
  GlobalModelSettingsFormValues,
  'thinking_model_blacklist' | 'chat_completions_to_responses_policy'
> & {
  thinking_model_blacklist: string[]
  chat_completions_to_responses_policy: z.infer<typeof chatPolicy>
}
const migrationDraft: GlobalPolicy = {
  pass_through_request_enabled: false,
  thinking_model_blacklist: JSON.parse(thinkingBlacklistExample),
  chat_completions_to_responses_policy: {
    enabled: false,
    all_channels: false,
    channel_ids: [],
    model_patterns: [],
    overrides: {},
  },
  ping_interval_enabled: false,
  ping_interval_seconds: 60,
}

export function GlobalSettingsCard() {
  return (
    <NativePolicySection
      name='global_model_policy'
      draft={migrationDraft}
      editor={GlobalPolicyForm}
    />
  )
}

function GlobalPolicyForm({
  policy,
  preview,
  busy,
}: PolicyFormProps<GlobalPolicy>) {
  const { t } = useTranslation()
  const form = useForm<
    GlobalModelSettingsFormInput,
    unknown,
    GlobalModelSettingsFormValues
  >({
    resolver: zodResolver(schema),
    defaultValues: {
      ...policy,
      thinking_model_blacklist: JSON.stringify(
        policy.thinking_model_blacklist,
        null,
        2
      ),
      chat_completions_to_responses_policy: JSON.stringify(
        policy.chat_completions_to_responses_policy,
        null,
        2
      ),
    },
  })
  const pingEnabled = useWatch({
    control: form.control,
    name: 'ping_interval_enabled',
  })

  const onSubmit = (values: GlobalModelSettingsFormValues) =>
    preview({
      ...policy,
      ...values,
      thinking_model_blacklist: modelNames.parse(
        JSON.parse(values.thinking_model_blacklist)
      ),
      chat_completions_to_responses_policy: chatPolicy.parse(
        JSON.parse(values.chat_completions_to_responses_policy)
      ),
    })

  return (
    <SettingsSection title={t('Global Model Configuration')}>
      <Form {...form}>
        <SettingsForm onSubmit={form.handleSubmit(onSubmit)}>
          <SettingsPageFormActions
            onSave={form.handleSubmit(onSubmit)}
            isSaveDisabled={busy}
            saveLabel='Preview changes'
          />
          <FormField
            control={form.control}
            name='pass_through_request_enabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Enable Request Passthrough')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Passthrough requires matching client and upstream protocols and model names.'
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
            name='thinking_model_blacklist'
            render={({ field }) => (
              <FormItem>
                <FormLabel>
                  {t('Models that skip thinking suffix processing')}
                </FormLabel>
                <FormControl>
                  <SystemJsonCodeEditor
                    configurationKey='global.thinking_model_blacklist'
                    value={field.value}
                    onChange={(value) => field.onChange(value)}
                    name={field.name}
                    onBlur={field.onBlur}
                    textareaRef={field.ref}
                    placeholder={`${t('Example:')}\n${thinkingBlacklistExample}`}
                    heightClassName='h-32 min-h-32 max-h-32'
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'Models listed here will not automatically append or remove -thinking / -nothinking suffixes.'
                  )}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />

          <Separator />

          <div className='space-y-4'>
            <div className='flex items-center gap-2'>
              <h3 className='text-base font-semibold'>
                {t('ChatCompletions -> Responses Compatibility')}
              </h3>
              <StatusBadge
                label={t('Preview')}
                variant='neutral'
                copyable={false}
              />
            </div>

            <Alert>
              <AlertTitle>{t('Warning')}</AlertTitle>
              <AlertDescription>
                {t(
                  'This feature is experimental. Configuration format and behavior may change.'
                )}
              </AlertDescription>
            </Alert>

            <FormField
              control={form.control}
              name='chat_completions_to_responses_policy'
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t('Policy JSON')}</FormLabel>
                  <FormControl>
                    <SystemJsonCodeEditor
                      configurationKey='global.chat_completions_to_responses_policy'
                      value={field.value}
                      onChange={(value) => field.onChange(value)}
                      name={field.name}
                      onBlur={field.onBlur}
                      textareaRef={field.ref}
                      placeholder={`${t('Example (specific channels):')}\n${chatToResponsesPolicyExample}\n\n${t('Example (all channels):')}\n${chatToResponsesPolicyAllChannelsExample}`}
                    />
                  </FormControl>
                  <FormDescription>
                    {t(
                      'Channel overrides take precedence for matching models. Use a channel ID as the key and true or false as its value.'
                    )}
                  </FormDescription>
                  <div className='flex flex-wrap gap-2'>
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      onClick={() =>
                        form.setValue(
                          'chat_completions_to_responses_policy',
                          chatToResponsesPolicyExample,
                          { shouldDirty: true }
                        )
                      }
                    >
                      {t('Fill example (specific channels)')}
                    </Button>
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      onClick={() =>
                        form.setValue(
                          'chat_completions_to_responses_policy',
                          chatToResponsesPolicyAllChannelsExample,
                          { shouldDirty: true }
                        )
                      }
                    >
                      {t('Fill example (all channels)')}
                    </Button>
                  </div>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>

          <Separator />

          <FormField
            control={form.control}
            name='ping_interval_enabled'
            render={({ field }) => (
              <SettingsSwitchItem>
                <SettingsSwitchContent>
                  <FormLabel>{t('Keep-alive Ping')}</FormLabel>
                  <FormDescription>
                    {t(
                      'Send SSE comment heartbeats while waiting. WebSocket ping uses a separate transport setting.'
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
            name='ping_interval_seconds'
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t('Ping Interval (seconds)')}</FormLabel>
                <FormControl>
                  <Input
                    type='number'
                    min={1}
                    disabled={!pingEnabled}
                    className='w-24'
                    value={
                      field.value === undefined || field.value === null
                        ? ''
                        : String(field.value)
                    }
                    onChange={(event) => field.onChange(event.target.value)}
                    onBlur={field.onBlur}
                    name={field.name}
                    ref={field.ref}
                  />
                </FormControl>
                <FormDescription>
                  {t(
                    'Recommended to keep this high to avoid upstream throttling.'
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
