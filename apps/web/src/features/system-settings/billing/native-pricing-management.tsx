/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Dialog } from '@/components/dialog'
import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import {
  microsToUsd,
  usdToMicros,
} from '@/features/channels/components/native-model-draft'
import {
  getNativeConfiguration,
  nativeRequest,
  NATIVE_ROOT,
  readNativeRoutingScope,
  stringifyNativeDocument,
  type NativePriceDraft,
  type NativePrices,
  type NativeRoutingScope,
  type NativeVersion,
} from '@/features/channels/native-api'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  routingPriceCoverage,
  type PriceSelector,
} from './native-price-coverage'
import { NativePriceCoveragePanel } from './native-price-coverage-panel'
import { NativePriceRuleDialog } from './native-price-rule-dialog'
import {
  nextPriceRuleId,
  priceChanges,
  priceRateText,
  readPriceDocument,
  readPriceRules,
  samePriceRule,
  type NativePriceRule,
} from './native-price-rules'

type Draft = {
  expected: NativeVersion
  prices: NativePrices
  name: string
  id?: number
}
type Preview = {
  path: string
  method: 'PUT' | 'POST'
  body: Record<string, unknown>
  digest: string
  before: NativePrices
  after: NativePrices
}

export function NativePricingManagement({
  initialMode = '',
}: {
  initialMode?: string
}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeRoutingScope(auth)
  if (!scope) {
    return (
      <Alert variant='destructive'>
        <AlertDescription>
          {t('Administrator access required')}
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <PricingWorkspace
      key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
      scope={scope}
      initialMode={initialMode}
    />
  )
}

function PricingWorkspace({
  scope,
  initialMode,
}: {
  scope: NativeRoutingScope
  initialMode: string
}) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const query = useQuery({
    queryKey: [
      'native-routing-configuration',
      scope.userId,
      scope.sessionId,
      scope.role,
    ],
    queryFn: () => getNativeConfiguration(scope),
    gcTime: 0,
    refetchOnWindowFocus: false,
    retry: false,
  })
  const [search, setSearch] = useState('')
  const [group, setGroup] = useState('')
  const [mode, setMode] = useState(initialMode)
  const [page, setPage] = useState(1)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [editing, setEditing] = useState<{
    rule: NativePriceRule
    previousId?: number
  } | null>(null)
  const [json, setJson] = useState<string | null>(null)
  const [batchMultiplier, setBatchMultiplier] = useState('1')
  const [preview, setPreview] = useState<Preview | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [discard, setDiscard] = useState(false)
  const [deleteDraft, setDeleteDraft] = useState<number | null>(null)
  const pending = useRef(false)
  const configuration = query.data
  const prices = draft?.prices ?? configuration?.prices
  const fail = (cause: unknown) =>
    setError(
      getServerErrorMessage(cause) ??
        (cause instanceof Error ? cause.message : t('Failed to save'))
    )

  async function run(operation: () => Promise<void>) {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setError(null)
    try {
      await operation()
    } catch (cause) {
      fail(cause)
    } finally {
      pending.current = false
      setBusy(false)
    }
  }
  async function refresh() {
    await query.refetch()
    await client.invalidateQueries({ queryKey: ['pricing'] })
  }
  function updatePrices(next: NativePrices) {
    if (!configuration) return
    readPriceRules(next)
    setDraft({
      expected: draft?.expected ?? configuration.version,
      prices: {
        ...next,
        version: (draft?.expected ?? configuration.version).pricing + 1,
      },
      name: draft?.name ?? '',
      id: draft?.id,
    })
    setPreview(null)
    setError(null)
  }
  function saveRule(rule: NativePriceRule) {
    if (!prices || !editing) return
    try {
      const next =
        editing.previousId === undefined
          ? [...prices.rules, rule]
          : prices.rules.map((item) =>
              item.id === editing.previousId ? rule : item
            )
      updatePrices({ ...prices, rules: next })
      setEditing(null)
    } catch (cause) {
      fail(cause)
      throw cause
    }
  }
  function newRule(
    selector: PriceSelector,
    source: NativePrices
  ): NativePriceRule {
    return {
      id: nextPriceRuleId(source),
      ...selector,
      rates: {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        request: 0,
      },
      group_multiplier_ppm: 1_000_000,
      speed_multiplier_ppm: 1_000_000,
    }
  }
  function addRule(selector?: PriceSelector) {
    if (!configuration || !prices) return
    const rule = newRule(
      selector ?? {
        model: '',
        group: group || configuration.routing.groups[0]?.name || 'default',
        speed: 'standard',
      },
      prices
    )
    if (!selector && mode === 'upstream_cost') {
      rule.billing_mode = 'upstream_cost'
    }
    setEditing({ rule })
  }
  function saveToolFee(
    row: { selector: PriceSelector; rule?: NativePriceRule },
    micros: number
  ) {
    if (!prices) return
    try {
      const rule = row.rule ?? newRule(row.selector, prices)
      const next = { ...rule, rates: { ...rule.rates, request: micros } }
      updatePrices({
        ...prices,
        rules: row.rule
          ? prices.rules.map((item) => (item.id === rule.id ? next : item))
          : [...prices.rules, next],
      })
    } catch (cause) {
      fail(cause)
    }
  }
  async function previewChanges(saved?: NativePriceDraft) {
    if (!configuration) return
    const expected = saved ? configuration.version : draft?.expected
    const after = saved?.prices ?? draft?.prices
    if (!expected || !after) return
    const body = saved
      ? { expected_version: expected }
      : { expected_version: expected, prices: after }
    const path = saved
      ? `${NATIVE_ROOT}/prices/drafts/${saved.id}`
      : `${NATIVE_ROOT}/prices`
    await run(async () => {
      const response = await nativeRequest<{ confirmation_digest: string }>(
        scope,
        'POST',
        `${path}/preview`,
        body
      )
      if (!/^[0-9a-f]{64}$/.test(response.confirmation_digest)) {
        throw new Error(t('The preview did not return a confirmation digest.'))
      }
      setPreview({
        path: saved ? `${path}/publish` : path,
        method: saved ? 'POST' : 'PUT',
        body,
        digest: response.confirmation_digest,
        before: configuration.prices,
        after,
      })
    })
  }
  async function publish() {
    if (!preview) return
    const confirmed = preview
    await run(async () => {
      const receipt = await nativeRequest<{ version: NativeVersion }>(
        scope,
        confirmed.method,
        confirmed.path,
        { ...confirmed.body, confirmation_digest: confirmed.digest }
      )
      if (receipt.version.pricing !== confirmed.after.version) {
        throw new Error(
          t('The save result is unknown. Reload before continuing.')
        )
      }
      setPreview(null)
      setDraft(null)
      toast.success(t('Prices published successfully'))
      await refresh()
    })
  }
  async function saveDraft() {
    if (!draft || !configuration) return
    if (!draft.name.trim()) {
      setError(t('Enter a draft name.'))
      return
    }
    await run(async () => {
      const id =
        draft.id ??
        Object.values(configuration.bindings.price_drafts).reduce(
          (highest, item) => Math.max(highest, item.id),
          0
        ) + 1
      await nativeRequest(scope, 'PUT', `${NATIVE_ROOT}/prices/drafts/${id}`, {
        expected_version: draft.expected,
        name: draft.name.trim(),
        prices: draft.prices,
      })
      setDraft(null)
      toast.success(t('Draft saved. Published prices are unchanged.'))
      await refresh()
    })
  }
  async function removeDraft(id: number) {
    if (!configuration) return
    await run(async () => {
      await nativeRequest(
        scope,
        'DELETE',
        `${NATIVE_ROOT}/prices/drafts/${id}`,
        { expected_version: configuration.version }
      )
      setDeleteDraft(null)
      await refresh()
    })
  }
  if (query.isPending) return <LoadingState />
  if (query.isError && !configuration) {
    return (
      <ErrorState
        description={
          getServerErrorMessage(query.error) ?? t('Failed to load data')
        }
        onRetry={() => void query.refetch()}
      />
    )
  }
  if (!configuration || !prices) return null
  let rules: NativePriceRule[]
  try {
    rules = readPriceRules(prices)
  } catch (cause) {
    return (
      <ErrorState
        description={
          cause instanceof Error ? cause.message : t('Invalid configuration')
        }
        onRetry={() => void query.refetch()}
      />
    )
  }
  const needle = search.trim().toLowerCase()
  const groups = [
    ...new Set([
      ...configuration.routing.groups.map((item) => item.name),
      ...rules.map((rule) => rule.group),
    ]),
  ].sort()
  const coverage = routingPriceCoverage(configuration.routing, rules)
  const matching = rules.filter(
    (rule) =>
      (!group || rule.group === group) &&
      (!mode || (rule.billing_mode ?? 'tokens') === mode) &&
      (!needle ||
        [rule.model, rule.group, rule.speed, String(rule.id)].some((value) =>
          value.toLowerCase().includes(needle)
        ))
  )
  const lastPage = Math.max(1, Math.ceil(matching.length / 20))
  const currentPage = Math.min(page, lastPage)
  const visible = matching.slice((currentPage - 1) * 20, currentPage * 20)
  const changed = priceChanges(configuration.prices, prices)
  const dirtyCount = changed.added + changed.changed + changed.removed
  const stale =
    !!draft &&
    (draft.expected.routing !== configuration.version.routing ||
      draft.expected.pricing !== configuration.version.pricing)
  const drafts = Object.values(configuration.bindings.price_drafts)
  const billingLabel = (value: string | undefined) =>
    value === 'tiered_decimal'
      ? t('Tiered pricing')
      : value === 'upstream_cost'
        ? t('Upstream cost')
        : t('Token pricing')
  const download = () => {
    const url = URL.createObjectURL(
      new Blob([stringifyNativeDocument(prices, 2)], {
        type: 'application/json',
      })
    )
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `model-prices-v${prices.version}.json`
    anchor.click()
    URL.revokeObjectURL(url)
  }
  const modelPrices = (
    <>
      <div className='flex flex-wrap items-start justify-between gap-5'>
        <div className='space-y-2'>
          <p className='text-muted-foreground max-w-2xl text-sm leading-6'>
            {t(
              'Set model prices in USD, including input, output, cache and request fees. Group and speed multipliers apply to these base prices.'
            )}
          </p>
          <div className='text-muted-foreground flex flex-wrap gap-x-5 gap-y-2 text-xs tabular-nums'>
            <span>
              {t('Models')}: {new Set(rules.map((rule) => rule.model)).size}
            </span>
            <span>
              {t('Groups')}: {groups.length}
            </span>
            <span>
              {t('Price rules')}: {rules.length}
            </span>
            <span>
              {t('Published version')}: {configuration.version.pricing}
            </span>
          </div>
        </div>
        <div className='flex flex-wrap gap-2'>
          <Button
            type='button'
            variant='ghost'
            disabled={busy || query.isFetching}
            onClick={() => void refresh()}
          >
            {t('Refresh')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={busy}
            onClick={download}
          >
            {t('Export JSON')}
          </Button>
          <Button type='button' disabled={busy} onClick={() => addRule()}>
            {t('Add price rule')}
          </Button>
        </div>
      </div>
      <div className='flex min-w-0 flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-center'>
        <Input
          className='min-w-0 sm:max-w-sm'
          aria-label={t('Search models')}
          placeholder={t('Search models')}
          value={search}
          onChange={(event) => {
            setSearch(event.target.value)
            setPage(1)
          }}
        />
        <NativeSelect
          className='w-full sm:w-auto'
          aria-label={t('Group')}
          value={group}
          onChange={(event) => {
            setGroup(event.target.value)
            setPage(1)
          }}
        >
          <NativeSelectOption value=''>{t('All groups')}</NativeSelectOption>
          {groups.map((name) => (
            <NativeSelectOption value={name} key={name}>
              {name}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <NativeSelect
          className='w-full sm:w-auto'
          aria-label={t('Billing mode')}
          value={mode}
          onChange={(event) => {
            setMode(event.target.value)
            setPage(1)
          }}
        >
          <NativeSelectOption value=''>
            {t('All billing modes')}
          </NativeSelectOption>
          <NativeSelectOption value='tokens'>
            {t('Token pricing')}
          </NativeSelectOption>
          <NativeSelectOption value='tiered_decimal'>
            {t('Tiered pricing')}
          </NativeSelectOption>
          <NativeSelectOption value='upstream_cost'>
            {t('Upstream cost')}
          </NativeSelectOption>
        </NativeSelect>
        <Button
          type='button'
          variant='ghost'
          className='sm:ml-auto'
          disabled={busy}
          onClick={() => setJson(stringifyNativeDocument(prices, 2))}
        >
          {t('Edit JSON')}
        </Button>
      </div>
      {group && (
        <div className='bg-muted/40 flex flex-wrap items-end gap-3 rounded-lg p-4'>
          <label className='min-w-0 space-y-2 text-sm'>
            <span>{t('Group multiplier')}</span>
            <Input
              className='w-40 tabular-nums'
              inputMode='decimal'
              value={batchMultiplier}
              onChange={(event) => setBatchMultiplier(event.target.value)}
            />
          </label>
          <Button
            type='button'
            variant='outline'
            disabled={busy}
            onClick={() => {
              const multiplier = usdToMicros(batchMultiplier)
              if (
                multiplier === null ||
                multiplier > 1_000_000_000 ||
                (multiplier === 0 &&
                  rules.some(
                    (rule) =>
                      rule.group === group &&
                      rule.billing_mode === 'upstream_cost'
                  ))
              ) {
                setError(
                  t(
                    'Enter a valid multiplier from 0 to 1000. Upstream-cost rules require a value above zero.'
                  )
                )
                return
              }
              updatePrices({
                ...prices,
                rules: rules.map((rule) =>
                  rule.group === group
                    ? { ...rule, group_multiplier_ppm: multiplier }
                    : rule
                ),
              })
            }}
          >
            {t('Apply to group')}
          </Button>
          <span className='text-muted-foreground pb-2 text-sm'>
            {t('Applies to every price rule in {{group}}.', { group })}
          </span>
        </div>
      )}
      <div className='min-w-0'>
        <Table className='min-w-[66rem]'>
          <TableHeader>
            <TableRow>
              <TableHead>{t('Model')}</TableHead>
              <TableHead>
                {t('Group')} / {t('Speed')}
              </TableHead>
              <TableHead>{t('Input')}</TableHead>
              <TableHead>{t('Output')}</TableHead>
              <TableHead>{t('Cache read')}</TableHead>
              <TableHead>{t('Cache write')}</TableHead>
              <TableHead>{t('Request fee')}</TableHead>
              <TableHead>{t('Multiplier')}</TableHead>
              <TableHead className='bg-background sticky right-0 z-10 w-24 text-right'>
                {t('Actions')}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((rule) => (
              <TableRow key={rule.id}>
                <TableCell className='max-w-64 whitespace-normal'>
                  <div className='font-mono font-medium break-words'>
                    {rule.model}
                  </div>
                  <div className='text-muted-foreground mt-1 text-xs'>
                    {billingLabel(rule.billing_mode)}
                  </div>
                </TableCell>
                <TableCell>
                  <div>{rule.group}</div>
                  <div className='text-muted-foreground mt-1 text-xs'>
                    {rule.speed}
                  </div>
                </TableCell>
                {(
                  [
                    'input',
                    'output',
                    'cache_read',
                    'cache_write',
                    'request',
                  ] as const
                ).map((key) => (
                  <TableCell key={key} className='tabular-nums'>
                    {priceRateText(rule, key) ?? '—'}
                  </TableCell>
                ))}
                <TableCell className='tabular-nums'>
                  <div>
                    {microsToUsd(rule.group_multiplier_ppm)} ×{' '}
                    {microsToUsd(rule.speed_multiplier_ppm)}
                  </div>
                  <div className='text-muted-foreground mt-1 text-xs'>
                    {t('Group')} × {t('Speed')}
                  </div>
                </TableCell>
                <TableCell className='bg-background sticky right-0 z-10 w-24 text-right'>
                  <DropdownMenu>
                    <DropdownMenuTrigger
                      disabled={busy}
                      render={
                        <Button
                          type='button'
                          variant='ghost'
                          size='sm'
                          className='min-h-11 sm:min-h-8'
                        />
                      }
                    >
                      {t('Actions')}
                    </DropdownMenuTrigger>
                    <DropdownMenuContent
                      align='end'
                      className='bg-background min-w-40'
                    >
                      <DropdownMenuItem
                        disabled={busy}
                        onClick={() =>
                          setEditing({ rule, previousId: rule.id })
                        }
                      >
                        {t('Edit')}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={busy}
                        onClick={() =>
                          setEditing({
                            rule: {
                              ...rule,
                              id: nextPriceRuleId(prices),
                              model: `${rule.model}-copy`,
                            },
                          })
                        }
                      >
                        {t('Copy')}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={busy}
                        variant='destructive'
                        onClick={() =>
                          updatePrices({
                            ...prices,
                            rules: prices.rules.filter(
                              (item) => item.id !== rule.id
                            ),
                          })
                        }
                      >
                        {t('Remove')}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        {visible.length === 0 && (
          <div
            role='status'
            className='text-muted-foreground py-14 text-center text-sm'
          >
            {rules.length
              ? t('No matching entries')
              : t('No price rules yet. Add a model price to get started.')}
          </div>
        )}
        <p className='text-muted-foreground mt-3 text-xs leading-5'>
          {t(
            'Token prices are USD per 1M tokens; request fees are USD per request. Tier prices follow the configured tier order. A dash means that dimension does not apply.'
          )}
        </p>
      </div>
      <div className='flex flex-wrap items-center justify-between gap-3 text-sm'>
        <span className='text-muted-foreground'>
          {t('Total')}: {matching.length}
        </span>
        <div className='flex items-center gap-3'>
          <Button
            type='button'
            variant='ghost'
            disabled={currentPage === 1}
            onClick={() => setPage(currentPage - 1)}
          >
            {t('Previous')}
          </Button>
          <span className='tabular-nums'>
            {currentPage} / {lastPage}
          </span>
          <Button
            type='button'
            variant='ghost'
            disabled={currentPage === lastPage}
            onClick={() => setPage(currentPage + 1)}
          >
            {t('Next')}
          </Button>
        </div>
      </div>
    </>
  )
  return (
    <section
      className='@container/prices min-w-0 space-y-7'
      aria-label={t('Model Pricing')}
    >
      {error && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {query.isError && configuration && (
        <Alert variant='destructive'>
          <AlertDescription>
            {t('Failed to refresh prices. Your draft has been kept.')}
          </AlertDescription>
        </Alert>
      )}
      {stale && (
        <Alert role='status'>
          <AlertDescription>
            {t(
              'Published configuration changed. Keep your draft for reference, reload the latest prices and review your changes again.'
            )}
          </AlertDescription>
        </Alert>
      )}
      <Tabs defaultValue='models'>
        {!initialMode && (
          <TabsList variant='navigation' aria-label={t('Model Pricing')}>
            <TabsTrigger value='models'>{t('Model prices')}</TabsTrigger>
            <TabsTrigger value='missing'>{t('Unset price models')}</TabsTrigger>
            <TabsTrigger value='tools'>{t('Tool prices')}</TabsTrigger>
          </TabsList>
        )}
        <TabsContent value='models' className='space-y-7'>
          {modelPrices}
        </TabsContent>
        <TabsContent value='missing'>
          <NativePriceCoveragePanel
            rows={coverage.missing.map((selector) => ({ selector }))}
            disabled={busy || stale || query.isError}
            addRule={addRule}
            saveFee={saveToolFee}
          />
        </TabsContent>
        <TabsContent value='tools'>
          <NativePriceCoveragePanel
            rows={coverage.tools}
            tools
            disabled={busy || stale || query.isError}
            addRule={addRule}
            saveFee={saveToolFee}
          />
        </TabsContent>
      </Tabs>
      {draft && (
        <section
          className='bg-muted/50 space-y-4 rounded-xl p-5'
          aria-label={t('Pending price changes')}
        >
          <div className='flex flex-wrap items-center justify-between gap-3'>
            <div className='flex flex-wrap items-center gap-3'>
              <h3 className='font-medium'>{t('Pending price changes')}</h3>
              <Badge variant='secondary'>{dirtyCount}</Badge>
            </div>
            <Button
              type='button'
              variant='ghost'
              disabled={busy}
              onClick={() => setDiscard(true)}
            >
              {t('Discard changes')}
            </Button>
          </div>
          <p className='text-muted-foreground text-sm'>
            {t(
              '{{added}} added, {{changed}} changed, {{removed}} removed.',
              changed
            )}
          </p>
          <div className='flex flex-wrap items-end gap-3'>
            <label className='min-w-0 flex-1 space-y-2 text-sm'>
              <span>{t('Draft name')}</span>
              <Input
                value={draft.name}
                maxLength={128}
                disabled={busy}
                onChange={(event) =>
                  setDraft({ ...draft, name: event.target.value })
                }
              />
            </label>
            <Button
              type='button'
              variant='outline'
              disabled={busy || stale}
              onClick={() => void saveDraft()}
            >
              {t('Save draft')}
            </Button>
            <Button
              type='button'
              disabled={busy || stale || dirtyCount === 0}
              onClick={() => void previewChanges()}
            >
              {t('Preview changes')}
            </Button>
          </div>
        </section>
      )}
      {drafts.length > 0 && (
        <section
          className='space-y-4 border-t pt-6'
          aria-label={t('Saved price drafts')}
        >
          <h3 className='font-medium'>{t('Saved price drafts')}</h3>
          <div className='divide-y'>
            {drafts.map((saved) => {
              const outdated =
                saved.base_pricing_version !== configuration.version.pricing
              return (
                <div
                  key={saved.id}
                  className='flex min-w-0 flex-wrap items-center justify-between gap-4 py-4'
                >
                  <div className='min-w-0'>
                    <p className='font-medium break-words'>{saved.name}</p>
                    <p className='text-muted-foreground mt-1 text-xs'>
                      {t('Price rules')}: {saved.prices.rules.length} ·{' '}
                      {outdated ? t('Outdated draft') : t('Ready to publish')}
                    </p>
                  </div>
                  <div className='flex flex-wrap gap-2'>
                    <Button
                      type='button'
                      variant='ghost'
                      disabled={busy || !!draft || outdated}
                      onClick={() =>
                        setDraft({
                          expected: configuration.version,
                          prices: saved.prices,
                          name: saved.name,
                          id: saved.id,
                        })
                      }
                    >
                      {t('Edit')}
                    </Button>
                    <Button
                      type='button'
                      variant='outline'
                      disabled={busy || !!draft || outdated}
                      onClick={() => void previewChanges(saved)}
                    >
                      {t('Preview')}
                    </Button>
                    <Button
                      type='button'
                      variant='ghost'
                      disabled={busy || !!draft}
                      onClick={() => setDeleteDraft(saved.id)}
                    >
                      {t('Delete')}
                    </Button>
                  </div>
                </div>
              )
            })}
          </div>
        </section>
      )}
      {editing && (
        <NativePriceRuleDialog
          initial={editing.rule}
          groups={groups}
          models={[
            ...new Set(
              configuration.routing.routes.map((route) => route.model)
            ),
          ]}
          onClose={() => setEditing(null)}
          onSave={saveRule}
        />
      )}
      <Dialog
        open={json !== null}
        onOpenChange={(open) => {
          if (!open) setJson(null)
        }}
        title={t('Edit JSON')}
        description={t(
          'Advanced edits preserve exact prices. Preview is required before publication.'
        )}
        footer={
          <>
            <Button type='button' variant='ghost' onClick={() => setJson(null)}>
              {t('Cancel')}
            </Button>
            <Button
              type='button'
              onClick={() => {
                try {
                  updatePrices(
                    readPriceDocument(
                      json ?? '',
                      (draft?.expected ?? configuration.version).pricing + 1
                    )
                  )
                  setJson(null)
                } catch (cause) {
                  fail(cause)
                }
              }}
            >
              {t('Apply to draft')}
            </Button>
          </>
        }
      >
        <Textarea
          aria-label={t('Price configuration JSON')}
          className='min-h-96 font-mono text-xs'
          value={json ?? ''}
          onChange={(event) => setJson(event.target.value)}
        />
        {error && (
          <p role='alert' className='text-destructive mt-3 text-sm'>
            {error}
          </p>
        )}
      </Dialog>
      <Dialog
        open={preview !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setPreview(null)
        }}
        title={t('Publish prices')}
        description={t(
          'Review these changes before they apply to new requests. Existing price snapshots remain unchanged.'
        )}
        contentClassName='sm:max-w-3xl'
        footer={
          <>
            <Button
              type='button'
              variant='ghost'
              disabled={busy}
              onClick={() => setPreview(null)}
            >
              {t('Cancel')}
            </Button>
            <Button
              type='button'
              disabled={busy}
              onClick={() => void publish()}
            >
              {busy ? t('Saving...') : t('Confirm and publish')}
            </Button>
          </>
        }
      >
        {preview && <PricePublicationReview preview={preview} />}
        {error && (
          <p role='alert' className='text-destructive mt-3 text-sm'>
            {error}
          </p>
        )}
      </Dialog>
      <Dialog
        open={discard || deleteDraft !== null}
        onOpenChange={(open) => {
          if (!open && !busy) {
            setDiscard(false)
            setDeleteDraft(null)
          }
        }}
        title={discard ? t('Discard changes') : t('Delete draft')}
        footer={
          <>
            <Button
              type='button'
              variant='ghost'
              disabled={busy}
              onClick={() => {
                setDiscard(false)
                setDeleteDraft(null)
              }}
            >
              {t('Cancel')}
            </Button>
            <Button
              type='button'
              variant='destructive'
              disabled={busy}
              onClick={() => {
                if (discard) {
                  setDraft(null)
                  setDiscard(false)
                  setPreview(null)
                  setError(null)
                } else if (deleteDraft !== null) void removeDraft(deleteDraft)
              }}
            >
              {discard ? t('Discard changes') : t('Confirm delete')}
            </Button>
          </>
        }
      >
        <p className='text-sm'>{t('Published prices will not change.')}</p>
      </Dialog>
    </section>
  )
}

function PricePublicationReview({ preview }: { preview: Preview }) {
  const { t } = useTranslation()
  const before = readPriceRules(preview.before)
  const after = readPriceRules(preview.after)
  const changes = after.filter(
    (rule) =>
      !samePriceRule(
        before.find((old) => old.id === rule.id),
        rule
      )
  )
  return (
    <div className='space-y-5'>
      <p className='text-sm'>
        {t(
          '{{added}} added, {{changed}} changed, {{removed}} removed.',
          priceChanges(preview.before, preview.after)
        )}
      </p>
      <div className='max-h-[55vh] space-y-6 overflow-auto'>
        {changes.map((rule) => {
          const previous = before.find((old) => old.id === rule.id)
          const rows = [
            [t('Group'), previous?.group, rule.group],
            [t('Speed'), previous?.speed, rule.speed],
            ...(
              [
                ['input', t('Input')],
                ['output', t('Output')],
                ['cache_read', t('Cache read')],
                ['cache_write', t('Cache write')],
                ['request', t('Request fee')],
              ] as const
            ).map(([key, label]) => [
              label,
              previous ? priceRateText(previous, key) : null,
              priceRateText(rule, key),
            ]),
            [
              t('Group multiplier'),
              previous ? microsToUsd(previous.group_multiplier_ppm) : null,
              microsToUsd(rule.group_multiplier_ppm),
            ],
            [
              t('Speed multiplier'),
              previous ? microsToUsd(previous.speed_multiplier_ppm) : null,
              microsToUsd(rule.speed_multiplier_ppm),
            ],
          ]
          return (
            <section key={rule.id} className='space-y-3'>
              <h4 className='font-mono text-sm font-medium break-words'>
                {rule.model}
              </h4>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t('Pricing')}</TableHead>
                    <TableHead>{t('Current')}</TableHead>
                    <TableHead>{t('New')}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map(([label, oldValue, newValue]) => (
                    <TableRow key={label}>
                      <TableCell>{label}</TableCell>
                      <TableCell>{oldValue ?? '—'}</TableCell>
                      <TableCell
                        className={oldValue !== newValue ? 'font-medium' : ''}
                      >
                        {newValue ?? '—'}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
              <details>
                <summary className='text-muted-foreground cursor-pointer text-xs'>
                  {t('Advanced Settings')}
                </summary>
                <pre className='bg-muted mt-2 overflow-auto rounded-md p-3 text-xs'>
                  {stringifyNativeDocument(rule, 2)}
                </pre>
              </details>
            </section>
          )
        })}
        {before
          .filter((rule) => !after.some((next) => next.id === rule.id))
          .map((rule) => (
            <p key={rule.id} className='text-destructive text-sm'>
              {t('Remove')}: {rule.model} · {rule.group} · {rule.speed}
            </p>
          ))}
      </div>
    </div>
  )
}
