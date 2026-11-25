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
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { PersonalBudgetPanel } from '@/features/teams/budget-panel'
import { TeamsPanel } from '@/features/teams/panel'
import { useStatus } from '@/hooks/use-status'
import { useAuthStore } from '@/stores/auth-store'

import { CheckinHistoryCard } from './components/checkin-history-card'
import { GiftCard } from './components/gift-card'
import { LanguagePreferencesCard } from './components/language-preferences-card'
import { LoginSessionsCard } from './components/login-sessions-card'
import { OAuthGrantsCard } from './components/oauth-grants-card'
import { PasskeyCard } from './components/passkey-card'
import {
  ProfileCheckinCapability,
  ProfileTwoFactorCapability,
} from './components/profile-feature-capabilities'
import { ProfileHeader } from './components/profile-header'
import { ProfileIdentityCard } from './components/profile-identity-card'
import { ProfileIPAllowlistCard } from './components/profile-ip-allowlist-card'
import { ProfileSecurityCard } from './components/profile-security-card'
import { ProfileSettingsCard } from './components/profile-settings-card'
import { ReferralOverviewCard } from './components/referral-overview-card'
import { SettlementCurrencyCard } from './components/settlement-currency-card'
import { SidebarModulesCard } from './components/sidebar-modules-card'
import { WalletDisplayCurrencyCard } from './components/wallet-display-currency-card'
import { useProfile } from './hooks'
import { profileSearchSchema } from './navigation'

interface ProfilePasskeyCapabilityProps {
  capabilitiesReady: boolean
  passkeyLogin: boolean
  loading: boolean
}

export function ProfilePasskeyCapability({
  capabilitiesReady,
  passkeyLogin,
  loading,
}: ProfilePasskeyCapabilityProps) {
  if (!capabilitiesReady || !passkeyLogin) return null
  return <PasskeyCard loading={loading} />
}

export function Profile() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const search = useSearch({ strict: false })
  const activeTab = profileSearchSchema.parse(search).tab ?? 'overview'
  const { profile, loading, error, fetchProfile, refreshProfile } = useProfile()
  const {
    status,
    capabilitiesReady,
    loading: statusLoading,
    refetch: refreshStatus,
  } = useStatus()
  const permissions = useAuthStore((s) => s.auth.user?.permissions)
  const sessionId = useAuthStore((s) => s.auth.session?.sid)
  const canConfigureSidebar = permissions?.sidebar_settings !== false

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Profile')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {error ? (
          <ErrorState
            title={t('Failed to load profile')}
            description={error}
            onRetry={() => void fetchProfile()}
          />
        ) : (
          <Tabs
            value={activeTab}
            onValueChange={(tab) =>
              void navigate({
                to: '/profile',
                search: profileSearchSchema.parse({ tab }),
              })
            }
            className='console-page-tabs mx-auto max-w-6xl pb-8'
          >
            <TabsList variant='navigation' aria-label={t('Profile')}>
              <TabsTrigger value='overview'>{t('Overview')}</TabsTrigger>
              <TabsTrigger value='account'>{t('Account & budget')}</TabsTrigger>
              <TabsTrigger value='security'>{t('Security')}</TabsTrigger>
              <TabsTrigger value='teams'>{t('Teams')}</TabsTrigger>
              <TabsTrigger value='preferences'>{t('Preferences')}</TabsTrigger>
              <TabsTrigger value='rewards'>{t('Rewards')}</TabsTrigger>
            </TabsList>
            <TabsContent value='overview'>
              <ProfileHeader profile={profile} loading={loading} />
            </TabsContent>
            <TabsContent value='account' className='space-y-7'>
              <ProfileIdentityCard
                key={`${String(profile?.id)}:${sessionId}`}
                profile={profile}
                loading={loading}
                onProfileUpdate={refreshProfile}
              />
              <PersonalBudgetPanel />
              <ProfileSettingsCard
                profile={profile}
                loading={loading}
                onProfileUpdate={refreshProfile}
              />
            </TabsContent>
            <TabsContent value='security' className='space-y-5'>
              <div className='grid items-start gap-5 lg:grid-cols-2'>
                <ProfileSecurityCard profile={profile} loading={loading} />
                <div className='space-y-5'>
                  <ProfileTwoFactorCapability
                    ready={capabilitiesReady}
                    enabled={status?.two_factor_enabled === true}
                    loading={statusLoading}
                    onRefresh={() => void refreshStatus()}
                  />
                  <ProfilePasskeyCapability
                    capabilitiesReady={capabilitiesReady}
                    passkeyLogin={status?.passkey_login === true}
                    loading={loading}
                  />
                </div>
              </div>
              <ProfileIPAllowlistCard />
              <LoginSessionsCard />
              <OAuthGrantsCard />
            </TabsContent>
            <TabsContent value='teams'>
              <TeamsPanel />
            </TabsContent>
            <TabsContent value='preferences' className='space-y-5'>
              <div className='grid items-start gap-5 lg:grid-cols-2'>
                <LanguagePreferencesCard
                  profile={profile}
                  onProfileUpdate={refreshProfile}
                />
                <WalletDisplayCurrencyCard
                  profile={profile}
                  loading={loading}
                  onProfileUpdate={refreshProfile}
                />
                <SettlementCurrencyCard
                  profile={profile}
                  loading={loading}
                  onProfileUpdate={refreshProfile}
                />
              </div>
              {canConfigureSidebar && <SidebarModulesCard />}
            </TabsContent>
            <TabsContent
              value='rewards'
              className='grid items-start gap-5 lg:grid-cols-2'
            >
              <GiftCard
                capabilitiesReady={capabilitiesReady}
                enabled={status?.compensation_gifts_enabled}
                onRefresh={() => void refreshStatus()}
              />
              <ReferralOverviewCard />
              <CheckinHistoryCard />
              <ProfileCheckinCapability
                ready={capabilitiesReady}
                enabled={status?.checkin_enabled === true}
                loading={statusLoading}
                onRefresh={() => void refreshStatus()}
              />
            </TabsContent>
          </Tabs>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
