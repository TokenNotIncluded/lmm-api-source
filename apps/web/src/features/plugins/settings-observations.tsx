/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react'
import { useTranslation } from 'react-i18next'

import { getServerErrorMessage } from '@/lib/server-error-message'

const features = {
  toolmarket: ['toolReview'],
  store: ['storeReview', 'merchantConfig'],
  operations: ['scripts', 'deployment'],
  checkins: ['rewards'],
  payments: [
    'checkout',
    'pancake',
    'epay',
    'stripe',
    'creem',
    'waffo',
    'coinbase',
  ],
  acquisition: ['links'],
  assistant: ['assistantRoute'],
  'hero-sms': ['activations'],
} as const
type Feature = (typeof features)[keyof typeof features][number]
type Result =
  | 'missingApi'
  | 'readFailed'
  | 'noFunding'
  | 'noRoute'
  | 'needsSetup'
  | 'off'
  | 'configured'
  | 'available'
  | 'funded'
  | 'noPurchases'
type Observation = { result: Result; checkedAt: number; writable?: boolean }
type Observations = Partial<Record<Feature, Observation>>
type Report = (feature: Feature, observation: Observation) => void
export const PluginObservationContext = createContext<Report | null>(null)

// Owned by the currently mounted administrator session. No request or secret
// is stored here; each panel reports only facts from its existing read.
export function usePluginObservations() {
  const [observations, setObservations] = useState<Observations>({})
  const report = useCallback<Report>((feature, observation) => {
    setObservations((current) => {
      const old = current[feature]
      if (old && old.checkedAt > observation.checkedAt) return current
      if (
        old?.checkedAt === observation.checkedAt &&
        old.result === observation.result &&
        old.writable === observation.writable
      )
        return current
      return { ...current, [feature]: observation }
    })
  }, [])
  return { observations, report }
}

export function useReportPluginSetting(
  feature: Feature,
  result: Result | undefined,
  checkedAt: number,
  writable?: boolean
) {
  const report = useContext(PluginObservationContext)
  useEffect(() => {
    if (report && result && checkedAt > 0) {
      report(feature, { result, checkedAt, writable })
    }
  }, [report, feature, result, checkedAt, writable])
}

export function pluginReadTime(query: {
  isFetchedAfterMount: boolean
  dataUpdatedAt: number
  errorUpdatedAt: number
}) {
  return query.isFetchedAfterMount
    ? Math.max(query.dataUpdatedAt, query.errorUpdatedAt)
    : 0
}

export function settingsReadFailure(
  error: unknown
): 'missingApi' | 'readFailed' {
  const reason = getServerErrorMessage(error)?.trim().toLowerCase()
  return (isAxiosError(error) && error.response?.status === 404) ||
    reason === 'api endpoint not found'
    ? 'missingApi'
    : 'readFailed'
}

export function PluginSettingsObservations({
  name,
  observations,
  enabled,
}: {
  name: string
  observations: Observations
  enabled: boolean
}) {
  const { t } = useTranslation()
  const owner: readonly Feature[] = Object.hasOwn(features, name)
    ? features[name as keyof typeof features]
    : []
  const labels: Record<Feature, string> = {
    toolReview: t('Market AI first review'),
    storeReview: t('Market AI first review'),
    merchantConfig: t('Merchant store'),
    scripts: t('Published scripts'),
    deployment: t('io.net Deployments'),
    rewards: t('Check-in rewards'),
    checkout: t('Payment gateways'),
    pancake: t('Waffo Pancake'),
    epay: t('Epay Gateway'),
    stripe: 'Stripe',
    creem: 'Creem',
    waffo: 'Waffo',
    coinbase: 'Coinbase',
    links: t('Promotion links'),
    assistantRoute: t('AI assistant'),
    activations: t('Temporary activations'),
  }
  const results: Record<Result, string> = {
    missingApi: t('Settings endpoint not found'),
    readFailed: t('Failed to load settings'),
    noFunding: t('Reward funding unavailable'),
    noPurchases: t('Purchases unavailable'),
    noRoute: t('The shared AI review route is unavailable'),
    needsSetup: t('Not configured'),
    off: t('Disabled'),
    configured: t('Configured'),
    available: t('Available'),
    funded: t('Reward funding available'),
  }
  const observed = owner.filter((feature) => observations[feature])
  if (!observed.length) {
    return enabled ? (
      <p className='text-muted-foreground text-xs'>
        {t('Service availability unverified')}
      </p>
    ) : null
  }
  return (
    <dl className='grid gap-2 text-sm'>
      {observed.map((feature) => {
        const item = observations[feature]!
        return (
          <div key={feature}>
            <dt className='font-medium'>{labels[feature]}</dt>
            <dd>
              {results[item.result]}
              {item.writable !== undefined && (
                <>
                  {' '}
                  · {item.writable ? t('Settings editable') : t('Read-only')}
                </>
              )}
              <small className='text-muted-foreground ms-2'>
                {t('Last checked: {{time}}', {
                  time: new Date(item.checkedAt).toLocaleString(),
                })}
              </small>
            </dd>
          </div>
        )
      })}
    </dl>
  )
}
