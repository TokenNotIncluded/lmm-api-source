/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UseFormReturn } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from '@/components/ui/field'
import {
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

import {
  SettingsFormGrid,
  SettingsSwitchContent,
  SettingsSwitchItem,
} from '../components/settings-form-layout'
import {
  TRUST_LEVEL_BENEFIT_CODES,
  type TrustLevelBenefitsConfig,
} from './trust-level-benefits-schema'

const benefitLabels = {
  standard_access: 'Standard access',
  developer_access: 'Developer console access',
  usage_discount: 'Usage discount',
} as const
const roleBenefitLabels = {
  administrator_access: 'Administrator access',
  superadministrator_access: 'Super administrator access',
  usage_discount: 'Usage discount',
} as const

export function TrustLevelBenefitsFields({
  form,
  defaultValues,
  saving,
  exactRatios = false,
}: {
  form: UseFormReturn<TrustLevelBenefitsConfig>
  defaultValues: TrustLevelBenefitsConfig
  saving: boolean
  exactRatios?: boolean
}) {
  const { t } = useTranslation()
  return (
    <>
      <SettingsFormGrid>
        <FormField
          control={form.control}
          name='paid_activation_enabled'
          render={({ field }) => (
            <SettingsSwitchItem>
              <SettingsSwitchContent>
                <FormLabel>{t('Let a recharge unlock the console')}</FormLabel>
                <FormDescription>
                  {t(
                    'When enabled, reaching the configured L1 recharge threshold activates developer access. Invitation access and assistant activation remain separate.'
                  )}
                </FormDescription>
              </SettingsSwitchContent>
              <FormControl>
                <Switch
                  checked={field.value}
                  onCheckedChange={field.onChange}
                  disabled={saving}
                />
              </FormControl>
            </SettingsSwitchItem>
          )}
        />
        <FormField
          control={form.control}
          name='decay_period_days'
          render={({ field }) => (
            <FormItem>
              <FormLabel>{t('Inactivity review period (days)')}</FormLabel>
              <FormControl>
                <Input
                  type='number'
                  min={0}
                  max={3650}
                  step={1}
                  {...field}
                  disabled={saving}
                  onChange={(event) =>
                    field.onChange(
                      event.currentTarget.value === ''
                        ? ''
                        : event.currentTarget.valueAsNumber
                    )
                  }
                />
              </FormControl>
              <FormDescription>
                {t(
                  'Set 0 to disable inactivity decay. Administrator roles do not decay.'
                )}
              </FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
      </SettingsFormGrid>
      <FieldGroup className='gap-6'>
        {defaultValues.tiers.map((tier, index) => (
          <FieldSet key={tier.level} className='gap-3'>
            <FieldLegend>L{tier.level}</FieldLegend>
            <SettingsFormGrid>
              {tier.level === 0 ? (
                <p className='text-muted-foreground text-sm'>
                  {t('L0 has no recharge threshold.')}
                </p>
              ) : (
                <FormField
                  control={form.control}
                  name={`tiers.${index}.min_paid_credits`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {t(
                          'Cumulative recharge required for L{{level}} (credits)',
                          { level: tier.level }
                        )}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type='number'
                          min={1}
                          max={Number.MAX_SAFE_INTEGER}
                          step={1}
                          {...field}
                          disabled={saving}
                          onChange={(event) =>
                            field.onChange(
                              event.currentTarget.value === ''
                                ? ''
                                : event.currentTarget.valueAsNumber
                            )
                          }
                        />
                      </FormControl>
                      <FormDescription>
                        {t(
                          'Cumulative successful eligible recharges, not the extra amount for this step.'
                        )}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}
              <FormField
                control={form.control}
                name={`tiers.${index}.discount_ratio`}
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      {t(
                        exactRatios
                          ? 'L{{level}} charge multiplier'
                          : 'L{{level}} usage discount (%)',
                        {
                          level: tier.level,
                        }
                      )}
                    </FormLabel>
                    <FormControl>
                      <Input
                        type='number'
                        min={0}
                        max={exactRatios ? 1 : 99.999999}
                        step='any'
                        name={field.name}
                        ref={field.ref}
                        onBlur={field.onBlur}
                        value={
                          typeof field.value === 'number' &&
                          Number.isFinite(field.value)
                            ? exactRatios
                              ? field.value
                              : Number(((1 - field.value) * 100).toFixed(10))
                            : ''
                        }
                        disabled={saving}
                        onChange={(event) =>
                          field.onChange(
                            event.currentTarget.value === ''
                              ? Number.NaN
                              : exactRatios
                                ? event.currentTarget.valueAsNumber
                                : 1 - event.currentTarget.valueAsNumber / 100
                          )
                        }
                      />
                    </FormControl>
                    <FormDescription>
                      {t(
                        exactRatios
                          ? '1 charges the full usage price; 0.9 charges 90%. Recharge amounts are unchanged.'
                          : '0% uses the standard usage price. This discount does not change recharge amounts.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name={`tiers.${index}.benefits`}
                render={({ field }) => (
                  <FormItem data-settings-form-span='full'>
                    <FormLabel>{t('Displayed benefits')}</FormLabel>
                    <FieldGroup className='gap-3 sm:flex-row sm:flex-wrap'>
                      {TRUST_LEVEL_BENEFIT_CODES.map((code) => {
                        const id = `level-${tier.level}-benefit-${code}`
                        return (
                          <Field
                            key={code}
                            orientation='horizontal'
                            className='w-auto'
                            data-disabled={saving}
                          >
                            <Checkbox
                              id={id}
                              checked={(field.value ?? []).includes(code)}
                              disabled={saving}
                              onCheckedChange={(checked) =>
                                field.onChange(
                                  checked
                                    ? [...(field.value ?? []), code]
                                    : (field.value ?? []).filter(
                                        (benefit) => benefit !== code
                                      )
                                )
                              }
                            />
                            <FieldLabel htmlFor={id}>
                              {t(benefitLabels[code])}
                            </FieldLabel>
                          </Field>
                        )
                      })}
                    </FieldGroup>
                    <FormDescription>
                      {t(
                        'Benefit labels describe this level. Developer access follows activation rules; administrator permissions follow account roles.'
                      )}
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </SettingsFormGrid>
            {tier.level < 4 && <Separator />}
          </FieldSet>
        ))}
      </FieldGroup>
      <Alert>
        <AlertTitle>{t('Role levels (L5–L6)')}</AlertTitle>
        <AlertDescription>
          <p>{t('L5: Administrator. L6: Super administrator.')}</p>
          <p>
            {t(
              'Assigned through account roles only. There are no recharge thresholds for L5 or L6.'
            )}
          </p>
        </AlertDescription>
      </Alert>
      {defaultValues.role_tiers ? (
        <FieldGroup className='gap-6'>
          {defaultValues.role_tiers.map((tier, index) => (
            <FieldSet key={tier.level} className='gap-3'>
              <FieldLegend>
                L{tier.level} ·{' '}
                {t(tier.level === 5 ? 'Administrator' : 'Super administrator')}
              </FieldLegend>
              <SettingsFormGrid>
                <FormField
                  control={form.control}
                  name={`role_tiers.${index}.discount_ratio`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {t(
                          exactRatios
                            ? 'L{{level}} charge multiplier'
                            : 'L{{level}} usage discount (%)',
                          {
                            level: tier.level,
                          }
                        )}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type='number'
                          min={0}
                          max={exactRatios ? 1 : 99.999999}
                          step='any'
                          name={field.name}
                          ref={field.ref}
                          onBlur={field.onBlur}
                          value={
                            typeof field.value === 'number' &&
                            Number.isFinite(field.value)
                              ? exactRatios
                                ? field.value
                                : Number(((1 - field.value) * 100).toFixed(10))
                              : ''
                          }
                          disabled={saving}
                          onChange={(event) =>
                            field.onChange(
                              event.currentTarget.value === ''
                                ? Number.NaN
                                : exactRatios
                                  ? event.currentTarget.valueAsNumber
                                  : 1 - event.currentTarget.valueAsNumber / 100
                            )
                          }
                        />
                      </FormControl>
                      <FormDescription>
                        {t(
                          'Role discounts are configured independently of recharge levels.'
                        )}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name={`role_tiers.${index}.benefits`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t('Displayed benefits')}</FormLabel>
                      <FieldGroup className='gap-3'>
                        {(
                          [
                            tier.level === 5
                              ? 'administrator_access'
                              : 'superadministrator_access',
                            'usage_discount',
                          ] as const
                        ).map((code) => {
                          const id = `role-level-${tier.level}-benefit-${code}`
                          return (
                            <Field
                              key={code}
                              orientation='horizontal'
                              data-disabled={saving}
                            >
                              <Checkbox
                                id={id}
                                checked={(field.value ?? []).includes(code)}
                                disabled={saving}
                                onCheckedChange={(checked) =>
                                  field.onChange(
                                    checked
                                      ? [...(field.value ?? []), code]
                                      : (field.value ?? []).filter(
                                          (benefit) => benefit !== code
                                        )
                                  )
                                }
                              />
                              <FieldLabel htmlFor={id}>
                                {t(roleBenefitLabels[code])}
                              </FieldLabel>
                            </Field>
                          )
                        })}
                      </FieldGroup>
                      <FormDescription>
                        {t(
                          'Editing role benefits does not assign or remove account roles.'
                        )}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </SettingsFormGrid>
            </FieldSet>
          ))}
        </FieldGroup>
      ) : (
        <Alert>
          <AlertDescription>
            {t(
              'Reload server settings to edit the independent L5 and L6 role benefits.'
            )}
          </AlertDescription>
        </Alert>
      )}
    </>
  )
}
