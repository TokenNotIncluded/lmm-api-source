/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import type { NativeUserStatistics } from '@/features/native-users/contract'
import { toIntlLocale } from '@/i18n/languages'

function Statistic({ label, value }: { label: string; value: string }) {
  return (
    <div className='min-w-0 space-y-1.5'>
      <dt className='text-muted-foreground text-xs leading-relaxed'>{label}</dt>
      <dd className='font-mono text-xl font-medium break-all tabular-nums'>
        {value}
      </dd>
    </div>
  )
}

/** Financial positions, normalized usage and original source counters stay distinct. */
export function UserStatistics({
  statistics,
}: {
  statistics: NativeUserStatistics | undefined
}) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  const number = (value: string | null | undefined) =>
    value === null || value === undefined
      ? t('Unavailable')
      : new Intl.NumberFormat(locale).format(BigInt(value))
  const date = (value: number) =>
    new Intl.DateTimeFormat(locale, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(value)
  const usage = statistics?.usage
  const history = statistics?.historical_counters

  return (
    <div className='space-y-7'>
      <section
        className='space-y-4'
        aria-label={t('Recorded model activity')}
        data-testid='actor-user-statistics'
      >
        <h3 className='text-sm font-medium'>{t('Recorded model activity')}</h3>
        {usage?.status === 'available' ? (
          <>
            <dl className='grid grid-cols-1 gap-x-6 gap-y-5 min-[380px]:grid-cols-2'>
              <Statistic label={t('Requests')} value={number(usage.requests)} />
              <Statistic label={t('Tokens')} value={number(usage.tokens)} />
              <Statistic
                label={t('Usage (normalized credits)')}
                value={number(usage.credits)}
              />
              <Statistic
                label={t('Models used')}
                value={number(usage.model_count)}
              />
            </dl>
            <div className='text-muted-foreground space-y-1.5 text-xs leading-relaxed'>
              <p>
                {t(
                  "Counts this user's settled requests, including team usage and available imported history. Refunds are deducted."
                )}
              </p>
              <p>
                {t(
                  'These activity totals overlap personal account usage and are not additional charges.'
                )}
              </p>
              <p>{t('500,000 normalized credits = 1 USD.')}</p>
              {!usage.pricing_available && (
                <p>
                  {t(
                    'Some history has no verified price; its usage cost is unavailable.'
                  )}
                </p>
              )}
              <p>
                {t('Observed at {{time}}', {
                  time: date(usage.observed_at_unix_ms),
                })}
              </p>
            </div>
          </>
        ) : (
          <p className='text-muted-foreground text-sm'>{t('Unavailable')}</p>
        )}
      </section>

      <section className='space-y-4' aria-label={t('Invitation statistics')}>
        <h3 className='text-sm font-medium'>{t('Invitation statistics')}</h3>
        <dl className='grid grid-cols-1 gap-x-6 gap-y-5 min-[380px]:grid-cols-2'>
          <Statistic
            label={t('Recorded invitees')}
            value={number(statistics?.referrals.invited_users)}
          />
          <Statistic
            label={t('Affiliate balance (Core credits)')}
            value={number(statistics?.referrals.affiliate_balance_units)}
          />
          <Statistic
            label={t('Cumulative granted rewards (normalized credits)')}
            value={number(
              statistics?.referrals.reviewed_rewards?.granted_credits
            )}
          />
        </dl>
        <p className='text-muted-foreground text-xs leading-relaxed'>
          {t(
            'Counts distinct users in available invitation records. The reward balance may include debt and does not guarantee it can be claimed.'
          )}
        </p>
        {statistics?.referrals.reviewed_rewards && (
          <div className='text-muted-foreground space-y-1.5 text-xs leading-relaxed'>
            <p>
              {t(
                'Includes reviewed imported rewards and later settlements. Revoked rewards do not reduce this cumulative figure.'
              )}
            </p>
            <p>{t('500,000 normalized credits = 1 USD.')}</p>
          </div>
        )}
      </section>

      <details className='text-sm' data-testid='imported-user-counters'>
        <summary className='text-muted-foreground focus-visible:outline-ring min-h-11 cursor-pointer py-3 break-words focus-visible:outline-2'>
          {t('Imported account counters')}
        </summary>
        {history?.status === 'available' ? (
          <div className='space-y-4 pt-2'>
            <dl className='grid grid-cols-1 gap-x-6 gap-y-5 min-[380px]:grid-cols-2'>
              <Statistic
                label={t('Historical usage (source units)')}
                value={number(history.used_quota)}
              />
              <Statistic
                label={t('Historical request count')}
                value={number(history.request_count)}
              />
              <Statistic
                label={t('Historical rewards (source units)')}
                value={number(history.affiliate_rewards)}
              />
            </dl>
            <div className='text-muted-foreground space-y-1.5 text-xs leading-relaxed'>
              <p>
                {t(
                  'Historical counters retain their original units and are shown separately from usage records.'
                )}
              </p>
              <p>
                {t('Observed at {{time}}', {
                  time: date(history.observed_at_unix_ms),
                })}
              </p>
            </div>
          </div>
        ) : (
          <p className='text-muted-foreground py-2'>
            {history?.status === 'not_imported'
              ? t('No imported account counters are available.')
              : t('Unavailable')}
          </p>
        )}
      </details>
    </div>
  )
}
