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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useParams } from '@tanstack/react-router'
import { Suspense, useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

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
import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { PageFooterPortal } from '@/components/layout/components/page-footer'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

import './settings-workspace.css'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'

import {
  settingsUseRestrictedView,
  writableOptionKey,
} from '../hooks/option-writability'
import { useSystemOptions, getOptionValue } from '../hooks/use-system-options'
import {
  parseHeaderNavModules,
  serializeHeaderNavModules,
} from '../maintenance/config'
import type { SystemOption, SystemOptionsCapabilities } from '../types'
import { lazyNamedSection } from '../utils/lazy-section'
import { getSettingsErrorMessage } from '../utils/settings-error-message'
import { SettingsPageProvider } from './settings-page-context'
import { SettingsBreadcrumb, SettingsSearch } from './settings-search'

type SettingsPageProps<
  TSettings extends Record<string, string | number | boolean | unknown[]>,
  TSectionId extends string,
  TExtraArgs extends unknown[] = [],
> = {
  routePath: string
  defaultSettings: TSettings
  defaultSection: TSectionId
  getSectionContent: (
    sectionId: TSectionId,
    settings: TSettings,
    ...extraArgs: TExtraArgs
  ) => ReactNode
  getSectionMeta: (sectionId: TSectionId) => {
    titleKey: string
  }
  extraArgs?: TExtraArgs
  loadingMessage?: string
  resolveSettings?: (
    settings: TSettings,
    raw: SystemOption[] | undefined
  ) => TSettings
  /** Sections that load and save through their own authenticated API. */
  independentSections?: readonly TSectionId[]
}

type SettingsPageFrameProps = {
  title: ReactNode
  children: ReactNode
  writesBlocked?: boolean
}

export function SettingsPageFrame(props: SettingsPageFrameProps) {
  const [formActionsContainer, setFormActionsContainer] =
    useState<HTMLDivElement | null>(null)
  const [actionsContainer, setActionsContainer] =
    useState<HTMLDivElement | null>(null)
  const [titleStatusContainer, setTitleStatusContainer] =
    useState<HTMLSpanElement | null>(null)

  return (
    <SettingsPageProvider
      actionsContainer={actionsContainer}
      formActionsContainer={formActionsContainer}
      titleStatusContainer={titleStatusContainer}
    >
      <SectionPageLayout className='settings-workspace'>
        <SectionPageLayout.Breadcrumb>
          <SettingsBreadcrumb />
        </SectionPageLayout.Breadcrumb>
        <SectionPageLayout.Title>
          <span className='inline-flex max-w-full min-w-0 flex-wrap items-center gap-3 align-middle'>
            <span className='settings-page-title'>{props.title}</span>
            <span
              ref={setTitleStatusContainer}
              className='inline-flex min-w-0 shrink-0 items-center'
            />
          </span>
        </SectionPageLayout.Title>
        <SectionPageLayout.Actions>
          <SettingsSearch />
          <div
            ref={setActionsContainer}
            inert={props.writesBlocked}
            className='flex flex-wrap items-center justify-end gap-2'
          />
        </SectionPageLayout.Actions>
        <SectionPageLayout.Content>
          <div className='settings-sheet'>{props.children}</div>
          <PageFooterPortal>
            <div
              className='settings-save-dock'
              role='group'
              aria-label={
                typeof props.title === 'string' ? props.title : undefined
              }
            >
              <span className='settings-save-context text-muted-foreground text-sm'>
                {props.title}
              </span>
              <div
                ref={setFormActionsContainer}
                inert={props.writesBlocked}
                className='settings-save-target flex flex-wrap items-center justify-end gap-2'
              />
            </div>
          </PageFooterPortal>
        </SectionPageLayout.Content>
      </SectionPageLayout>
    </SettingsPageProvider>
  )
}

/** Show only returned values, without mounting editors or filling missing defaults. */
const LimitedNavigationSection = lazyNamedSection(
  () => import('../maintenance/header-navigation-section'),
  'HeaderNavigationSection'
)
const LimitedNoticeSection = lazyNamedSection(
  () => import('../maintenance/notice-section'),
  'NoticeSection'
)
const LimitedAnnouncementsSection = lazyNamedSection(
  () => import('../content/announcements-section'),
  'AnnouncementsSection'
)
const LimitedPricingSection = lazyNamedSection(
  () => import('../general/pricing-section'),
  'PricingSection'
)
const LimitedIPAccessSection = lazyNamedSection(
  () => import('../security/ip-access-routing-section'),
  'IPAccessRoutingSection'
)
const LimitedOAuthRegistrationPolicy = lazyNamedSection(
  () => import('../auth/oauth-registration-policy'),
  'OAuthRegistrationPolicy'
)

function LimitedSettings({
  options,
  readOnly,
  partial,
  activeSection,
  capabilities,
}: {
  options: SystemOption[]
  readOnly: boolean
  partial: boolean
  activeSection: string
  capabilities?: SystemOptionsCapabilities
}) {
  // @context-fold begin limited-settings-view
  const { t } = useTranslation()
  const fields: Record<string, { section: string; label: string }> = {
    ServerAddress: { section: 'system-info', label: t('Server Address') },
    RegisterEnabled: {
      section: 'basic-auth',
      label: t('Registration Enabled'),
    },
    PasswordRegisterEnabled: {
      section: 'basic-auth',
      label: t('Password Registration'),
    },
    OAuthRegisterEnabled: {
      section: 'basic-auth',
      label: t('OAuth Registration'),
    },
    RegistrationDisabledMethods: {
      section: 'basic-auth',
      label: t('Disabled registration methods'),
    },
    HeaderNavModules: {
      section: 'header-navigation',
      label: t('Header navigation'),
    },
    Notice: { section: 'notice', label: t('System Notice') },
    Announcements: {
      section: 'announcements',
      label: t('Announcements'),
    },
    'console_setting.announcements': {
      section: 'announcements',
      label: t('Announcements'),
    },
    'console_setting.announcements_enabled': {
      section: 'announcements',
      label: t('Enable Announcements'),
    },
    USDExchangeRate: {
      section: 'currency',
      label: t('USD Exchange Rate'),
    },
    IPAccessRoutingRules: {
      section: 'ip-access-routing',
      label: t('IP access routing'),
    },
    AssistantPersona: {
      section: 'assistant',
      label: t('Assistant persona'),
    },
  }
  const navigationOption = options.find(
    (option) => option.key === 'HeaderNavModules'
  )
  const writableNavigation =
    !readOnly &&
    activeSection === 'header-navigation' &&
    navigationOption !== undefined &&
    writableOptionKey(capabilities, 'HeaderNavModules') === 'HeaderNavModules'
  const writableNotice =
    !readOnly &&
    activeSection === 'notice' &&
    options.some((option) => option.key === 'Notice') &&
    writableOptionKey(capabilities, 'Notice') === 'Notice'
  const announcementOption =
    options.find((option) => option.key === 'console_setting.announcements') ??
    options.find((option) => option.key === 'Announcements')
  const writableAnnouncements =
    !readOnly &&
    activeSection === 'announcements' &&
    announcementOption !== undefined &&
    writableOptionKey(capabilities, announcementOption.key) !== undefined
  const exchangeRateOption = options.find(
    (option) => option.key === 'USDExchangeRate'
  )
  const writableExchangeRate =
    !readOnly &&
    activeSection === 'currency' &&
    exchangeRateOption !== undefined &&
    writableOptionKey(capabilities, 'USDExchangeRate') === 'USDExchangeRate'
  const ipAccessOption = options.find(
    (option) => option.key === 'IPAccessRoutingRules'
  )
  const writableIPAccess =
    !readOnly &&
    activeSection === 'ip-access-routing' &&
    ipAccessOption !== undefined &&
    writableOptionKey(capabilities, 'IPAccessRoutingRules') ===
      'IPAccessRoutingRules'
  const visibleOptions = options.filter((option) => {
    if (fields[option.key]?.section !== activeSection) return false
    if (writableExchangeRate && option.key === 'USDExchangeRate') return false
    if (writableIPAccess && option.key === 'IPAccessRoutingRules') return false
    if (writableNavigation && option.key === 'HeaderNavModules') return false
    if (writableNotice && option.key === 'Notice') return false
    if (
      option.key === 'Announcements' ||
      option.key === 'console_setting.announcements'
    ) {
      return !writableAnnouncements && option === announcementOption
    }
    return true
  })
  const hasEditor =
    writableNavigation ||
    writableNotice ||
    writableAnnouncements ||
    writableExchangeRate ||
    writableIPAccess
  const hasContent =
    hasEditor || visibleOptions.length > 0 || activeSection === 'basic-auth'
  const deploymentNote =
    activeSection === 'system-info'
      ? t('The server address is managed in the deployment configuration.')
      : activeSection === 'basic-auth'
        ? t(
            'Registration availability is managed in the deployment configuration.'
          )
        : t('These settings are read-only on this server.')

  return (
    <div className='settings-available-content'>
      {partial && hasContent ? (
        <p className='settings-summary-note'>
          {t('Only settings supported by this server are shown.')}
        </p>
      ) : null}
      {writableNavigation && navigationOption ? (
        <LimitedNavigationSection
          config={parseHeaderNavModules(navigationOption.value)}
          initialSerialized={serializeHeaderNavModules(
            parseHeaderNavModules(navigationOption.value)
          )}
        />
      ) : null}
      {writableNotice ? (
        <LimitedNoticeSection
          defaultValue={
            options.find((option) => option.key === 'Notice')?.value ?? ''
          }
        />
      ) : null}
      {writableAnnouncements && announcementOption ? (
        <LimitedAnnouncementsSection
          enabled={(() => {
            const enabledOption = options.find(
              (option) => option.key === 'console_setting.announcements_enabled'
            )
            return enabledOption ? enabledOption.value === 'true' : undefined
          })()}
          enabledWritable={false}
          data={announcementOption.value}
        />
      ) : null}
      {writableExchangeRate && exchangeRateOption ? (
        <LimitedPricingSection
          defaultValues={{
            USDExchangeRate:
              exchangeRateOption.value === ''
                ? undefined
                : Number(exchangeRateOption.value),
          }}
        />
      ) : null}
      {writableIPAccess && ipAccessOption ? (
        <LimitedIPAccessSection
          defaultValues={{ IPAccessRoutingRules: ipAccessOption.value }}
        />
      ) : null}
      {visibleOptions.length > 0 ? (
        <section className='settings-server-values'>
          <div className='settings-readonly-heading'>
            <h3>{t('Current settings')}</h3>
            <span className='settings-readonly-label'>{t('Read only')}</span>
          </div>
          <p className='settings-summary-note'>{deploymentNote}</p>
          <dl className='settings-readonly-values'>
            {visibleOptions.map((option) => (
              <div key={option.key} className='settings-readonly-row'>
                <dt>{fields[option.key].label}</dt>
                <dd className='settings-readonly-value'>
                  {(option.key.endsWith('Enabled') ||
                    option.key.endsWith('_enabled')) &&
                  option.value === 'true'
                    ? t('Enabled')
                    : (option.key.endsWith('Enabled') ||
                          option.key.endsWith('_enabled')) &&
                        option.value === 'false'
                      ? t('Disabled')
                      : option.value || t('Not configured')}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      ) : null}
      {activeSection === 'basic-auth' ? (
        <LimitedOAuthRegistrationPolicy />
      ) : null}
      {!hasContent ? (
        <section className='settings-unavailable' role='status'>
          <span className='settings-readonly-label'>{t('Read only')}</span>
          <h3>{t('No settings available in this section')}</h3>
          <p>
            {activeSection === 'system-info'
              ? deploymentNote
              : t('This server does not expose controls for this section.')}
          </p>
        </section>
      ) : null}
    </div>
  )
  // @context-fold end limited-settings-view
}

/**
 * Generic settings page component
 * Handles loading state, data fetching, and section rendering
 */
export function SettingsPage<
  TSettings extends Record<string, string | number | boolean | unknown[]>,
  TSectionId extends string,
  TExtraArgs extends unknown[] = [],
>(props: SettingsPageProps<TSettings, TSectionId, TExtraArgs>) {
  const { t } = useTranslation()
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const params = useParams({ from: props.routePath as any })
  const section = (params?.section ?? props.defaultSection) as TSectionId

  if (props.independentSections?.includes(section)) {
    return (
      <SettingsPageFrame title={t(props.getSectionMeta(section).titleKey)}>
        <Suspense
          fallback={<LoadingState message={t('Loading settings...')} />}
        >
          {props.getSectionContent(
            section,
            props.defaultSettings,
            ...((props.extraArgs ?? []) as TExtraArgs)
          )}
        </Suspense>
      </SettingsPageFrame>
    )
  }

  return <OptionSettingsPage {...props} />
}

function OptionSettingsPage<
  TSettings extends Record<string, string | number | boolean | unknown[]>,
  TSectionId extends string,
  TExtraArgs extends unknown[] = [],
>({
  routePath,
  defaultSettings,
  defaultSection,
  getSectionContent,
  getSectionMeta,
  extraArgs,
  loadingMessage = 'Loading settings...',
  resolveSettings,
}: SettingsPageProps<TSettings, TSectionId, TExtraArgs>) {
  const { t } = useTranslation()
  const optionsQuery = useSystemOptions()
  const { data, isLoading, isError, error, isFetching, refetch } = optionsQuery
  const phase = useCapabilityFreshness(optionsQuery)
  const writesBlocked = phase !== 'fresh'
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const params = useParams({ from: routePath as any })
  const activeSection = (params?.section ?? defaultSection) as TSectionId
  const sectionMeta = getSectionMeta(activeSection)
  const readOnly = data?.capabilities?.read_only === true
  const partial = data?.capabilities?.partial === true
  const restricted =
    settingsUseRestrictedView(data?.capabilities) || readOnly || partial

  const settings = useMemo(() => {
    if (restricted) return null
    const baseSettings = getOptionValue(
      data?.data,
      defaultSettings
    ) as TSettings
    return resolveSettings
      ? resolveSettings(baseSettings, data?.data)
      : baseSettings
  }, [data?.data, defaultSettings, resolveSettings, restricted])

  if (isLoading && !data) {
    return (
      <SettingsPageFrame title={t(sectionMeta.titleKey)}>
        <LoadingState message={t(loadingMessage)} />
      </SettingsPageFrame>
    )
  }

  if (isError && !data) {
    return (
      <SettingsPageFrame title={t(sectionMeta.titleKey)}>
        <ErrorState
          title={t('Unable to load settings')}
          description={getSettingsErrorMessage(
            error,
            t('Failed to load settings')
          )}
          onRetry={() => {
            void refetch()
          }}
        />
      </SettingsPageFrame>
    )
  }

  const sectionContent =
    settings === null ? (
      <LimitedSettings
        options={data?.data ?? []}
        readOnly={readOnly}
        partial={partial}
        activeSection={activeSection}
        capabilities={data?.capabilities}
      />
    ) : (
      getSectionContent(
        activeSection,
        settings,
        ...((extraArgs ?? []) as TExtraArgs)
      )
    )

  return (
    <SettingsPageFrame
      title={t(sectionMeta.titleKey)}
      writesBlocked={writesBlocked}
    >
      {isError ? (
        <Alert className='mb-4'>
          <AlertTitle>{t('Unable to refresh settings')}</AlertTitle>
          <AlertDescription>
            <p>
              {t(
                'The last loaded settings and your current edits are still available.'
              )}
            </p>
            <p>
              {getSettingsErrorMessage(error, t('Failed to load settings'))}
            </p>
            <Button
              type='button'
              variant='outline'
              size='sm'
              className='mt-2 min-h-11 sm:min-h-9'
              disabled={isFetching}
              onClick={() => {
                void refetch()
              }}
            >
              {t('Retry')}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      {writesBlocked && !isError ? (
        <Alert className='mb-4' role='status'>
          <AlertDescription>
            <p>
              {t(
                'Capability information is not current. Refresh before making changes.'
              )}
            </p>
            <Button
              type='button'
              variant='outline'
              className='mt-2 min-h-11'
              disabled={isFetching}
              onClick={() => void refetch()}
            >
              {t('Refresh capabilities')}
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      {writesBlocked && isError ? (
        <p className='text-muted-foreground mb-4 text-sm'>
          {t(
            'Capability information is not current. Refresh before making changes.'
          )}
        </p>
      ) : null}
      <fieldset disabled={writesBlocked} className='min-w-0 border-0 p-0'>
        <Suspense fallback={<LoadingState message={t(loadingMessage)} />}>
          {sectionContent}
        </Suspense>
      </fieldset>
    </SettingsPageFrame>
  )
}
