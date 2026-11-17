/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { TitledCard } from '@/components/ui/titled-card'

import { getCheckinCapabilities } from '../api'
import { CheckinCalendarCard } from './checkin-calendar-card'
import { TwoFACard } from './two-fa-card'

interface CapabilityProps {
  ready: boolean
  enabled: boolean
  loading: boolean
  onRefresh: () => void
}

function CapabilityNotice({
  title,
  description,
  loading,
  onRefresh,
}: {
  title: string
  description: string
  loading: boolean
  onRefresh: () => void
}) {
  const { t } = useTranslation()
  return (
    <TitledCard title={title} description={description}>
      <Button variant='outline' disabled={loading} onClick={onRefresh}>
        {loading ? t('Loading...') : t('Retry')}
      </Button>
    </TitledCard>
  )
}

/** The public capability comes from Core's configured MFA keyring. */
export function ProfileTwoFactorCapability(props: CapabilityProps) {
  const { t } = useTranslation()
  if (!props.ready || !props.enabled) {
    return (
      <CapabilityNotice
        title={t('Two-Factor Authentication')}
        description={
          !props.ready
            ? t('Server capabilities have not been loaded.')
            : t('Two-factor authentication is not available on this server.')
        }
        loading={props.loading}
        onRefresh={props.onRefresh}
      />
    )
  }
  return <TwoFACard loading={false} />
}

/** Only query the Extensions check-in contract when the live status enables it. */
export function ProfileCheckinCapability(props: CapabilityProps) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: ['checkin', 'capabilities'],
    queryFn: getCheckinCapabilities,
    enabled: props.ready && props.enabled,
    retry: false,
    refetchOnWindowFocus: false,
  })
  if (!props.ready || !props.enabled || query.data?.enabled === false) {
    return (
      <CapabilityNotice
        title={t('Daily Check-in')}
        description={
          !props.ready
            ? t('Server capabilities have not been loaded.')
            : t('Check-in is not enabled on this server.')
        }
        loading={props.loading}
        onRefresh={props.onRefresh}
      />
    )
  }
  if (query.isError) {
    return (
      <ErrorState
        title={t('Unable to load check-in availability')}
        onRetry={() => void query.refetch()}
      />
    )
  }
  if (!query.data) return <p role='status'>{t('Loading...')}</p>
  return <CheckinCalendarCard checkinEnabled />
}
