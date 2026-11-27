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
import { Boxes, GitBranch, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { EntityQuickActions } from '@/components/entity-quick-actions'
import { GroupBadge } from '@/components/group-badge'
import { StatusBadge, type StatusBadgeProps } from '@/components/status-badge'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { useBillingUSD } from '@/hooks/use-billing-usd'
import { useHistoricalQuotaMoney } from '@/hooks/use-historical-quota-money'
import { getUserAvatarFallback, getUserAvatarStyle } from '@/lib/avatar'
import { formatTimestampToDate } from '@/lib/format'
import { getLobeIcon } from '@/lib/lobe-icon'
import { resolveModelProvider } from '@/lib/model-provider'
import type { ColumnDef } from '@/lib/table'
import { cn } from '@/lib/utils'

import { LOG_TYPE_ALL_VALUE } from '../../constants'
import type { UsageLog } from '../../data/schema'
import {
  formatModelName,
  getTieredBillingSummary,
  hasAnyCacheTokens,
  parseLogOther,
  isViolationFeeLog,
  renderAuditContent,
} from '../../lib/format'
import {
  formatLogPrice,
  formatLogTokenPrice,
  type LogCurrencyFormatter,
} from '../../lib/money'
import {
  missingFinalUsageReceipt,
  usageLogStatusCode,
  usageReceiptState,
  nativeRequestStates,
} from '../../lib/receipt-state'
import { getLogTiming } from '../../lib/timing'
import {
  isDisplayableLogType,
  isTimingLogType,
  getLogTypeConfig,
  isPerCallBilling,
} from '../../lib/utils'
import type { LogOtherData } from '../../types'
import { DetailsDialog } from '../dialogs/details-dialog'
import { LogCostDisplay } from '../log-cost-display'
import { LogUsageDisplay } from '../log-usage-display'
import { ModelBadge } from '../model-badge'
import { NativeRequestBadges } from '../native-request-badges'
import { TimingMetricsCell, StreamTpsCell } from '../timing-metrics-cell'
import { useUsageLogsContext } from '../usage-logs-provider'

type TFunction = {
  (key: string): string
  (key: string, options: Record<string, unknown>): string
}

interface DetailSegment {
  text: string
  muted?: boolean
  danger?: boolean
}

function formatRatioCompact(ratio: number | undefined): string {
  if (ratio == null || !Number.isFinite(ratio)) return '-'
  return ratio % 1 === 0
    ? String(ratio)
    : ratio.toFixed(4).replace(/\.?0+$/, '')
}

function getGroupRatio(other: LogOtherData | null): number | null {
  const userGroupRatio = other?.user_group_ratio
  if (
    userGroupRatio != null &&
    userGroupRatio !== -1 &&
    Number.isFinite(userGroupRatio)
  ) {
    return userGroupRatio
  }

  const groupRatio = other?.group_ratio
  if (groupRatio != null && groupRatio !== 1 && Number.isFinite(groupRatio)) {
    return groupRatio
  }

  return null
}

function getLegacyPricingAdjustment(other: LogOtherData | null): number | null {
  const multiplier = other?.dynamic_pricing
  if (multiplier == null || !Number.isFinite(multiplier) || multiplier <= 0) {
    return null
  }
  return multiplier
}

function buildDetailSegments(
  log: UsageLog,
  other: LogOtherData | null,
  t: TFunction,
  isAdmin: boolean,
  currency: LogCurrencyFormatter,
  formatQuota: (quota: number | null) => string
): DetailSegment[] {
  const segments = buildTypeDetailSegments(log, other, t, currency, formatQuota)
  // Quota saturation is a rare, admin-only anomaly marker; surface it first
  // and in danger styling so it stands out on the related billing log. The
  // backend already strips admin_info for non-admins; gate on isAdmin too as
  // defense in depth so the marker never leaks if that changes.
  if (isAdmin && other?.admin_info?.quota_saturation) {
    return [{ text: t('Quota clamped'), danger: true }, ...segments]
  }
  return segments
}

function buildTypeDetailSegments(
  log: UsageLog,
  other: LogOtherData | null,
  t: TFunction,
  currency: LogCurrencyFormatter,
  formatQuota: (quota: number | null) => string
): DetailSegment[] {
  // Audit (type=3) and login (type=7) logs: render localized content from the
  // structured op descriptor instead of the raw (English-fallback) content.
  if (log.type === 3 || log.type === 7) {
    const text = renderAuditContent(other, t, formatQuota)
    return text ? [{ text }] : []
  }

  if (log.type === 6) {
    return [{ text: t('Async task refund') }]
  }

  if (log.type !== 2) return []

  const isViolation = isViolationFeeLog(other)
  if (isViolation) {
    const segments: DetailSegment[] = []
    segments.push({ text: t('Violation Fee'), danger: true })
    if (other?.violation_fee_code) {
      segments.push({
        text: other.violation_fee_code,
        muted: true,
      })
    }
    segments.push({
      text: `${t('Fee')}: ${formatQuota(other?.fee_quota ?? log.quota)}`,
      muted: true,
    })
    return segments
  }

  if (!other) return []

  const segments: DetailSegment[] = []

  if (other.upstream_empty_usage) {
    segments.push({
      text: t('Upstream returned no usage; no quota charged'),
      danger: true,
    })
  }

  const isExpression = other.billing_mode === 'tiered_expr'
  const formatPriceCompact = (price: number) =>
    formatLogPrice(price, other, currency, isExpression)
  const formatPrice = (price: number) => `${formatPriceCompact(price)}/M`
  const formatPriceList = (prices: string[], showUnit: boolean) => {
    const text = prices.join(' / ')
    return showUnit ? `${text}/M` : text
  }
  const isTieredExpr = other.billing_mode === 'tiered_expr'
  const tieredSummary = getTieredBillingSummary(other)
  if (isTieredExpr) {
    if (tieredSummary) {
      const baseEntries = tieredSummary.priceEntries
        .filter((entry) => ['inputPrice', 'outputPrice'].includes(entry.field))
        .map((entry) => formatPriceCompact(entry.price))
      if (baseEntries.length > 0) {
        const tierLabel = tieredSummary.tier.label || t('Default')
        segments.push({
          text: `${tierLabel} · ${formatPriceList(baseEntries, true)}`,
        })
      }

      const cacheEntries = tieredSummary.priceEntries
        .filter((entry) =>
          ['cacheReadPrice', 'cacheCreatePrice', 'cacheCreate1hPrice'].includes(
            entry.field
          )
        )
        .map((entry) => {
          return formatPriceCompact(entry.price)
        })
      if (cacheEntries.length > 0) {
        segments.push({
          text: `${t('Cache')} ${formatPriceList(cacheEntries, false)}`,
          muted: true,
        })
      }

      const otherEntries = tieredSummary.priceEntries
        .filter(
          (entry) =>
            ![
              'inputPrice',
              'outputPrice',
              'cacheReadPrice',
              'cacheCreatePrice',
              'cacheCreate1hPrice',
            ].includes(entry.field)
        )
        .map((entry) => `${t(entry.shortLabel)} ${formatPrice(entry.price)}`)
      if (otherEntries.length > 0) {
        segments.push({
          text: otherEntries.join(' · '),
          muted: true,
        })
      }
    } else {
      // Parsing failure does not invalidate the tier recorded at settlement.
      // Preserve its label without inferring unit prices, including task logs.
      segments.push({
        text: `${t('Tiered pricing')} · ${other.matched_tier || t('Special billing expression')}`,
        muted: true,
      })
    }
  } else {
    const modelPrice = other.model_price
    const isPerCall = isPerCallBilling(modelPrice)
    if (isPerCall && modelPrice != null) {
      segments.push({
        text: `${t('Per-call')} · ${formatPriceCompact(modelPrice)}`,
      })
    } else if (other.model_ratio != null) {
      const tokenPrice = (multiplier: number) =>
        formatLogTokenPrice(other.model_ratio ?? 0, multiplier, other, currency)
      const baseEntries = [tokenPrice(1)]
      if (other.completion_ratio != null) {
        baseEntries.push(tokenPrice(other.completion_ratio))
      }
      segments.push({
        text: `${t('Standard')} · ${formatPriceList(baseEntries, true)}`,
      })

      if (hasAnyCacheTokens(other)) {
        const cacheEntries = [
          other.cache_ratio != null && other.cache_ratio !== 1
            ? tokenPrice(other.cache_ratio)
            : null,
          other.cache_creation_ratio != null && other.cache_creation_ratio !== 1
            ? tokenPrice(other.cache_creation_ratio)
            : null,
          other.cache_creation_ratio_1h != null &&
          other.cache_creation_ratio_1h !== 0
            ? tokenPrice(other.cache_creation_ratio_1h)
            : null,
        ].filter(Boolean) as string[]

        if (cacheEntries.length > 0) {
          segments.push({
            text: `${t('Cache')} ${formatPriceList(cacheEntries, false)}`,
            muted: true,
          })
        }
      }
    } else {
      const userGroupRatio = other.user_group_ratio
      const groupRatio = other.group_ratio
      const isUserGroup =
        userGroupRatio != null &&
        Number.isFinite(userGroupRatio) &&
        userGroupRatio !== -1
      const effectiveRatio = isUserGroup ? userGroupRatio : groupRatio
      const ratioLabel = isUserGroup
        ? t('User Exclusive Ratio')
        : t('Group Ratio')

      if (effectiveRatio != null && Number.isFinite(effectiveRatio)) {
        segments.push({
          text: `${ratioLabel} ${formatRatioCompact(effectiveRatio)}x`,
        })
      }
    }
  }

  if (other.is_system_prompt_overwritten) {
    segments.push({
      text: t('System Prompt Override'),
      danger: true,
    })
  }

  return segments
}

export function useCommonLogsColumns(isAdmin: boolean): ColumnDef<UsageLog>[] {
  const { t } = useTranslation()
  const currency = useBillingUSD()
  const columns: ColumnDef<UsageLog>[] = [
    {
      accessorKey: 'created_at',
      header: t('Time'),
      cell: ({ row }) => {
        const log = row.original
        const timestamp = row.getValue('created_at') as number
        const config = getLogTypeConfig(log.type)
        const other = parseLogOther(log.other)
        const receipt = usageReceiptState(log, other)
        const states = nativeRequestStates(log, other)
        const statusCode = usageLogStatusCode(log, other)

        return (
          <div className='flex min-w-0 flex-col gap-0.5'>
            <span className='truncate font-mono text-xs tabular-nums'>
              {formatTimestampToDate(timestamp)}
            </span>
            <div className='flex flex-wrap items-center gap-x-1'>
              {states ? (
                <NativeRequestBadges
                  states={states}
                  conversion={
                    isAdmin &&
                    log.route_attempts?.some(
                      (attempt) => attempt.protocol_path === 'converted'
                    )
                  }
                />
              ) : (
                <StatusBadge
                  label={t(receipt?.label ?? config.label)}
                  variant={
                    receipt?.variant ??
                    (config.color as StatusBadgeProps['variant'])
                  }
                  size='sm'
                  copyable={false}
                  className='-ml-1.5 !text-xs [&_span]:!text-xs'
                />
              )}
              {statusCode !== undefined && statusCode >= 400 && (
                <span
                  className='text-destructive font-mono text-xs'
                  title={t('Status Code')}
                >
                  {statusCode}
                </span>
              )}
            </div>
          </div>
        )
      },
      filterFn: (row, _id, value) => {
        if (!Array.isArray(value) || value.length === 0) return true
        if (value.includes(LOG_TYPE_ALL_VALUE)) return true
        return value.includes(String(row.original.type))
      },
      enableHiding: false,
      size: 145,
    },
  ]

  if (isAdmin) {
    columns.push(
      {
        id: 'channel',
        size: 76,
        header: t('Channel'),
        accessorFn: (row) => row.channel,
        cell: function ChannelCell({ row }) {
          const { sensitiveVisible, setAffinityTarget, setAffinityDialogOpen } =
            useUsageLogsContext()
          const log = row.original

          if (!isDisplayableLogType(log.type)) return null
          if (log.channel == null || log.channel === 0) {
            return <span className='text-muted-foreground'>—</span>
          }

          const other = parseLogOther(log.other)
          const affinity = other?.admin_info?.channel_affinity
          const rawUseChannel = other?.admin_info?.use_channel ?? []
          const useChannel = log.route_attempts?.length
            ? log.route_attempts.map((attempt) => String(attempt.channel_id))
            : Array.isArray(rawUseChannel)
              ? rawUseChannel.map(String).filter(Boolean)
              : []
          const hasRetryChain = useChannel.length > 1
          const channelChain = hasRetryChain
            ? useChannel.join(' → ')
            : undefined
          const channelDisplay = log.channel_name
            ? `${log.channel_name} #${log.channel}`
            : `#${log.channel}`
          const channelIdDisplay = `#${log.channel}`
          const channelName = sensitiveVisible ? log.channel_name : '••••'
          const multiKeyIndex = other?.admin_info?.multi_key_index
          const showMultiKeyIndex =
            other?.admin_info?.is_multi_key === true &&
            typeof multiKeyIndex === 'number' &&
            Number.isFinite(multiKeyIndex)

          return (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <div className='flex max-w-[160px] flex-col gap-0.5' />
                  }
                >
                  <div className='relative inline-flex w-fit items-center gap-1'>
                    <StatusBadge
                      label={channelIdDisplay}
                      autoColor={String(log.channel)}
                      copyText={String(log.channel)}
                      size='sm'
                      showDot={false}
                      className='font-mono'
                    />
                    {showMultiKeyIndex && (
                      <StatusBadge
                        label={String(multiKeyIndex)}
                        size='sm'
                        showDot={false}
                        copyable={false}
                        variant='neutral'
                        className='h-5 min-w-5 justify-center rounded-full px-1 font-mono text-xs'
                        aria-label={`${t('Key')} ${multiKeyIndex}`}
                      />
                    )}
                    {hasRetryChain && (
                      <Popover>
                        <PopoverTrigger
                          render={
                            <button
                              type='button'
                              className='text-muted-foreground hover:text-foreground focus-visible:ring-ring inline-flex size-5 shrink-0 items-center justify-center rounded-full transition-colors focus-visible:ring-2 focus-visible:outline-none'
                              aria-label={t('Retry Chain')}
                              onClick={(e) => e.stopPropagation()}
                            />
                          }
                        >
                          <GitBranch
                            className='console-status-warning-icon size-3.5'
                            aria-hidden='true'
                          />
                        </PopoverTrigger>
                        <PopoverContent
                          side='top'
                          align='start'
                          className='w-64 text-xs'
                        >
                          <div className='flex flex-col gap-1'>
                            <p className='font-medium'>{t('Retry Chain')}</p>
                            <p className='text-muted-foreground font-mono break-all'>
                              {channelChain}
                            </p>
                          </div>
                        </PopoverContent>
                      </Popover>
                    )}
                    {affinity && (
                      <button
                        type='button'
                        className='console-status-warning-text absolute -top-1 -right-1 leading-none'
                        onClick={(e) => {
                          e.stopPropagation()
                          setAffinityTarget({
                            rule_name: affinity.rule_name || '',
                            using_group:
                              affinity.using_group ||
                              affinity.selected_group ||
                              '',
                            key_hint: affinity.key_hint || '',
                            key_fp: affinity.key_fp || '',
                          })
                          setAffinityDialogOpen(true)
                        }}
                      >
                        <Sparkles className='size-3 fill-current' />
                      </button>
                    )}
                  </div>
                  {log.channel_name && (
                    <EntityQuickActions
                      kind='channel'
                      value={String(log.channel)}
                      label={channelName || channelIdDisplay}
                      hidden={!sensitiveVisible}
                    />
                  )}
                </TooltipTrigger>
                <TooltipContent>
                  <div className='space-y-1'>
                    <p>
                      {sensitiveVisible ? channelDisplay : channelIdDisplay}
                    </p>
                    {channelChain && (
                      <p className='text-muted-foreground text-xs'>
                        {t('Chain')}: {channelChain}
                      </p>
                    )}
                    {showMultiKeyIndex && (
                      <p className='text-muted-foreground text-xs'>
                        {t('Key')}: {multiKeyIndex}
                      </p>
                    )}
                    {affinity && (
                      <div className='border-t pt-1 text-xs'>
                        <p className='font-medium'>{t('Channel Affinity')}</p>
                        <p>
                          {t('Rule')}: {affinity.rule_name || '-'}
                        </p>
                        <p>
                          {t('Group')}:{' '}
                          {sensitiveVisible
                            ? affinity.using_group ||
                              affinity.selected_group ||
                              '-'
                            : '••••'}
                        </p>
                      </div>
                    )}
                  </div>
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )
        },
      },
      {
        id: 'user',
        size: 108,
        header: t('User'),
        accessorFn: (row) => row.username,
        cell: function UserCell({ row }) {
          const { sensitiveVisible, setSelectedUserId, setUserInfoDialogOpen } =
            useUsageLogsContext()
          const log = row.original

          if (!log.username) return null

          return (
            <div className='flex min-w-0 items-center gap-1.5'>
              <Avatar className='size-6 shrink-0'>
                <AvatarFallback
                  className='text-[11px] font-semibold'
                  style={
                    sensitiveVisible
                      ? getUserAvatarStyle(log.username)
                      : undefined
                  }
                >
                  {sensitiveVisible ? getUserAvatarFallback(log.username) : '•'}
                </AvatarFallback>
              </Avatar>
              <EntityQuickActions
                kind='username'
                value={log.username}
                label={sensitiveVisible ? log.username : '••••'}
                hidden={!sensitiveVisible}
                onDetails={() => {
                  setSelectedUserId(log.user_id)
                  setUserInfoDialogOpen(true)
                }}
              />
            </div>
          )
        },
      }
    )
  }

  columns.push({
    accessorKey: 'token_name',
    header: t('Token'),
    cell: function TokenNameCell({ row }) {
      const { sensitiveVisible } = useUsageLogsContext()
      const log = row.original
      if (!isDisplayableLogType(log.type)) return null

      const tokenName = log.token_name
      if (!tokenName) return null

      const other = parseLogOther(log.other)
      const displayName = sensitiveVisible ? tokenName : '••••'
      let group = log.group
      if (!group) group = other?.group || ''
      const groupRatio = getGroupRatio(other)
      const legacyPricingAdjustment = getLegacyPricingAdjustment(other)

      return (
        <div className='flex max-w-[200px] flex-col gap-0.5'>
          <TooltipProvider delay={300}>
            <Tooltip>
              <TooltipTrigger render={<div className='max-w-full' />}>
                <EntityQuickActions
                  kind='token'
                  value={tokenName}
                  label={displayName}
                  hidden={!sensitiveVisible}
                />
              </TooltipTrigger>
              {sensitiveVisible && tokenName.length > 16 && (
                <TooltipContent side='top' className='max-w-xs break-all'>
                  {tokenName}
                </TooltipContent>
              )}
            </Tooltip>
          </TooltipProvider>
          {(group || groupRatio != null || legacyPricingAdjustment != null) && (
            <span className='flex max-w-full flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-none'>
              {group ? (
                <GroupBadge
                  group={group}
                  label={sensitiveVisible ? undefined : '••••'}
                  type='text'
                  size='sm'
                  className='inline align-baseline text-xs leading-none [&>span]:leading-none'
                />
              ) : null}
              {group && groupRatio != null ? ' ' : null}
              {groupRatio != null ? (
                <span
                  className='text-muted-foreground/60 relative top-px align-baseline tabular-nums'
                  title={t('Cost multiplier')}
                  aria-label={`${t('Cost multiplier')}: ${formatRatioCompact(groupRatio)}x`}
                >
                  {t('Cost multiplier')} {formatRatioCompact(groupRatio)}x
                </span>
              ) : null}
              {legacyPricingAdjustment != null ? (
                <span
                  className='text-primary/80 relative top-px align-baseline tabular-nums'
                  title={t('Legacy pricing adjustment')}
                  aria-label={`${t('Legacy pricing adjustment')}: ${formatRatioCompact(legacyPricingAdjustment)}x`}
                >
                  {t('Legacy pricing adjustment')}{' '}
                  {formatRatioCompact(legacyPricingAdjustment)}x
                </span>
              ) : null}
            </span>
          )}
        </div>
      )
    },
    size: 118,
  })
  columns.push(
    {
      accessorKey: 'model_name',
      size: 128,
      header: t('Model'),
      cell: function ModelCell({ row }) {
        const log = row.original
        if (!isDisplayableLogType(log.type)) return null

        if (!log.model_name) return null
        const modelInfo = formatModelName(log)
        const provider = resolveModelProvider(modelInfo.name)

        return (
          <div className='flex w-fit max-w-full min-w-0 flex-col gap-0.5'>
            <EntityQuickActions
              kind='model'
              value={log.model_name}
              label={modelInfo.name}
              leading={
                <span
                  className='flex size-4 shrink-0 items-center justify-center'
                  aria-hidden='true'
                >
                  {provider?.icon ? (
                    getLobeIcon(provider.icon, 15)
                  ) : (
                    <Boxes className='size-3.5' />
                  )}
                </span>
              }
            />
            {(modelInfo.actualModel || modelInfo.responseModel) && (
              <ModelBadge
                modelName={modelInfo.name}
                actualModel={modelInfo.actualModel}
                responseModel={modelInfo.responseModel}
                detailsOnly
              />
            )}
          </div>
        )
      },
      meta: { mobileTitle: true },
    },
    {
      accessorKey: 'is_stream',
      size: 80,
      header: t('Stream'),
      cell: ({ row }) => {
        const log = row.original
        if (!isTimingLogType(log.type)) return null

        const other = parseLogOther(log.other)
        const timing = getLogTiming(log, other)

        return (
          <StreamTpsCell
            isStream={log.is_stream}
            tokensPerSecond={timing.throughput}
            generationSpeed={timing.generationSpeed}
            streamStatus={other?.stream_status}
          />
        )
      },
      meta: { label: t('Stream') },
    },
    {
      accessorKey: 'prompt_tokens',
      size: 104,
      header: t('Usage'),
      cell: ({ row }) => {
        const log = row.original
        if (!isDisplayableLogType(log.type)) return null

        const other = parseLogOther(log.other)

        const cacheReadTokens = other?.cache_tokens || 0
        const cacheWriteTokens =
          other?.cache_creation_tokens ??
          (other?.cache_creation_tokens_5m ?? 0) +
            (other?.cache_creation_tokens_1h ?? 0) +
            (other?.cache_creation_tokens_12h ?? 0)

        return (
          <div className='flex flex-col gap-0.5'>
            <span className='font-mono text-xs font-medium tabular-nums'>
              <LogUsageDisplay log={log} other={other} />
            </span>
            {(cacheReadTokens > 0 || cacheWriteTokens > 0) && (
              <div className='flex items-center gap-1 text-[11px]'>
                {cacheReadTokens > 0 && (
                  <span className='text-muted-foreground/60'>
                    {t('Cache')}↓ {cacheReadTokens.toLocaleString()}
                  </span>
                )}
                {cacheWriteTokens > 0 && (
                  <span className='text-muted-foreground/60'>
                    ↑ {cacheWriteTokens.toLocaleString()}
                  </span>
                )}
              </div>
            )}
          </div>
        )
      },
    },
    {
      accessorKey: 'quota',
      size: 100,
      header: t('Cost'),
      cell: ({ row }) => {
        const log = row.original
        if (!isDisplayableLogType(log.type)) return null

        const quota = log.quota
        const other = parseLogOther(log.other)
        return (
          <LogCostDisplay
            quota={quota}
            other={other}
            type={log.type}
            projection={log}
          />
        )
      },
    },

    {
      accessorKey: 'use_time',
      size: 104,
      header: t('Timing'),
      cell: ({ row }) => {
        const log = row.original
        if (!isTimingLogType(log.type)) return null

        const other = parseLogOther(log.other)
        const timing = getLogTiming(log, other)

        return (
          <TimingMetricsCell
            durationMs={timing.durationMs}
            completionTokens={log.completion_tokens}
            frtMs={timing.firstTokenMs}
            isStream={log.is_stream}
          />
        )
      },
    },

    {
      accessorKey: 'content',
      header: t('Details'),
      cell: function DetailsCell({ row }) {
        const [dialogOpen, setDialogOpen] = useState(false)
        const log = row.original
        const other = parseLogOther(log.other)

        const { formatHistoricalLogQuota, formatRawQuota } =
          useHistoricalQuotaMoney()
        const segments = buildDetailSegments(
          log,
          other,
          t,
          isAdmin,
          currency,
          (quota) =>
            log.type === 3 || log.type === 7
              ? formatRawQuota(quota)
              : formatHistoricalLogQuota({
                  quota: log.quota,
                  other,
                  type: log.type,
                  projection: log,
                  consumedQuota: quota,
                })
        )
        const primary = segments[0]
        const hasMore = segments.length > 1
        const fallbackDetail = missingFinalUsageReceipt(log, other)
          ? t('Final usage receipt unavailable')
          : log.content
        let primaryTextClass = 'text-foreground'
        if (primary?.muted) {
          primaryTextClass = 'text-muted-foreground/60'
        } else if (primary?.danger) {
          primaryTextClass = 'console-status-danger-text'
        }
        let detailPreview = <span className='text-muted-foreground/40'>—</span>
        if (primary) {
          detailPreview = (
            <span
              className={cn(
                'min-w-0 whitespace-normal break-words leading-snug group-hover:underline',
                primaryTextClass
              )}
            >
              {primary.text}
              {hasMore && (
                <span className='text-muted-foreground/40 ml-0.5'>
                  +{segments.length - 1}
                </span>
              )}
            </span>
          )
        } else if (fallbackDetail) {
          detailPreview = (
            <span className='text-muted-foreground min-w-0 break-words whitespace-normal group-hover:underline'>
              {fallbackDetail}
            </span>
          )
        }

        return (
          <>
            <button
              type='button'
              className='group flex min-h-8 w-full items-start gap-1.5 text-left text-xs'
              onClick={() => setDialogOpen(true)}
              title={t('Click to view full details')}
              aria-label={t('View request details')}
            >
              <span className='text-primary shrink-0'>{t('Details')}</span>
              <span className='flex min-w-0 items-start gap-1.5'>
                {detailPreview}
              </span>
            </button>
            <DetailsDialog
              log={log}
              isAdmin={isAdmin}
              open={dialogOpen}
              onOpenChange={setDialogOpen}
            />
          </>
        )
      },
      size: 360,
      maxSize: 640,
    }
  )

  return columns
}
