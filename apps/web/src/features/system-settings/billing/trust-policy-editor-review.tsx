/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

import type { TrustLevelBenefitsConfig } from './trust-level-benefits-schema'
import type { TrustNumericPolicy } from './trust-policy-editor-client'

const benefits = {
  standard_access: 'Standard access',
  developer_access: 'Developer console access',
  usage_discount: 'Usage discount',
  administrator_access: 'Administrator access',
  superadministrator_access: 'Super administrator access',
} as const

export function TrustPolicyReview({
  current,
  currentConsole,
  proposed,
  proposedConsole,
}: {
  current: TrustNumericPolicy
  currentConsole: TrustLevelBenefitsConfig | null
  proposed?: TrustNumericPolicy
  proposedConsole?: TrustLevelBenefitsConfig
}) {
  const { t } = useTranslation()
  const ratio = (value: TrustNumericPolicy['admin_discount']) =>
    `${value.numerator}/${value.denominator}`
  const descriptions = (
    policy: TrustLevelBenefitsConfig | null | undefined,
    level: number
  ) => {
    if (!policy) return t('Unknown')
    const codes =
      level < 5
        ? policy.tiers[level].benefits
        : policy.role_tiers?.[level - 5].benefits
    return codes?.length
      ? codes.map((code) => t(benefits[code])).join(', ')
      : t('None')
  }
  const rows: { label: string; before: string; after?: string }[] = [
    {
      label: t('Let a recharge unlock the console'),
      before: t(current.paid_activation_enabled ? 'Enabled' : 'Disabled'),
      after:
        proposed &&
        t(proposed.paid_activation_enabled ? 'Enabled' : 'Disabled'),
    },
    {
      label: t('Inactivity review period (days)'),
      before: String(current.decay_period_days),
      after: proposed && String(proposed.decay_period_days),
    },
  ]
  for (let level = 0; level <= 6; level++) {
    if (level > 0 && level < 5) {
      rows.push({
        label: `L${level} · ${t('Cumulative recharge threshold')}`,
        before: String(current.tiers[level].min_paid_credits),
        after: proposed && String(proposed.tiers[level].min_paid_credits),
      })
    }
    const discount = (policy: TrustNumericPolicy) =>
      level < 5
        ? policy.tiers[level].discount
        : level === 5
          ? policy.admin_discount
          : policy.superadmin_discount
    rows.push({
      label: `L${level} · ${t('Charge multiplier')}`,
      before: ratio(discount(current)),
      after: proposed && ratio(discount(proposed)),
    })
    rows.push({
      label: `L${level} · ${t('Benefits')}`,
      before: descriptions(currentConsole, level),
      after: proposed && descriptions(proposedConsole, level),
    })
  }
  return (
    <div className='space-y-2'>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Charge multipliers are exact fractions: 9/10 charges 90% of the base price. Role benefits never assign roles.'
        )}
      </p>
      <Table aria-label={t('Trust policy changes')}>
        <TableHeader>
          <TableRow>
            <TableHead>{t('Setting')}</TableHead>
            <TableHead>{t('Current')}</TableHead>
            {proposed && <TableHead>{t('Proposed')}</TableHead>}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <TableRow
              key={row.label}
              data-changed={row.after !== undefined && row.after !== row.before}
            >
              <TableCell>{row.label}</TableCell>
              <TableCell className='whitespace-normal'>{row.before}</TableCell>
              {proposed && (
                <TableCell
                  className={
                    row.after !== row.before
                      ? 'font-semibold whitespace-normal'
                      : 'text-muted-foreground whitespace-normal'
                  }
                >
                  {row.after}
                </TableCell>
              )}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}
