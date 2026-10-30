/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import {
  commerceScope,
  CommerceFailure,
  currentCommerceScope,
  formatCredits,
  nativeId,
  operationKey,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  archiveNativePlan,
  listNativePlans,
  planSchema,
  planTermsSchema,
  saveNativePlan,
  savePlanSchema,
  type NativePlan,
  type NativePlanTerms,
} from './native-api'

const commandSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('save'),
      id: nativeId.nullable(),
      request: savePlanSchema,
    })
    .strict(),
  z
    .object({ kind: z.literal('archive'), plan: planSchema, key: operationKey })
    .strict(),
])
type Command = z.infer<typeof commandSchema>
const initial: NativePlanTerms = {
  title: '',
  price_units: '500000',
  duration_seconds: 2592000,
  reset_period: 'grant',
  reset_seconds: 0,
  limit_units: '500000',
  models: ['*'],
  groups: ['*'],
  max_purchases_per_user: 0,
  enabled: false,
}
export function NativeSubscriptionAdmin() {
  const auth = useAuthStore((state) => state.auth)
  const scope = commerceScope()
  const { t } = useTranslation()
  if (!scope || (auth.user?.role ?? 0) < ROLE.SUPER_ADMIN) {
    return (
      <Alert>
        <AlertDescription>
          {t('Only super administrators can manage financial policies.')}
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <PlanAdmin key={`${auth.user?.id}:${auth.session?.sid}`} scope={scope} />
  )
}
function PlanAdmin({ scope }: { scope: CommerceScope }) {
  const { t, i18n } = useTranslation()
  const [plans, setPlans] = useState<NativePlan[]>([])
  const [editing, setEditing] = useState<NativePlan | null>(null)
  const [terms, setTerms] = useState<NativePlanTerms>(initial)
  const [archive, setArchive] = useState<NativePlan | null>(null)
  const [open, setOpen] = useState(false)
  const [pending, setPending] = useState<Command | null>(null)
  const [busy, setBusy] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [notice, setNotice] = useState('')
  const [refused, setRefused] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const current = () => mounted.current && currentCommerceScope(scope)
  async function load() {
    try {
      const result = await listNativePlans(scope, true)
      if (current()) setPlans(result)
    } catch {
      if (current()) setNotice(t('Request failed'))
    }
  }
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(scope, 'plan-admin', commandSchema)
      setPending(saved)
      if (saved) {
        setOpen(true)
        if (saved.kind === 'save') setTerms(saved.request.terms)
        else setArchive(saved.plan)
      }
    } catch {
      setStorageBlocked(true)
    }
    void load()
    return () => {
      mounted.current = false
    }
    // Remounted on account/session changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  function remember(value: Command | null) {
    try {
      savePending(scope, 'plan-admin', value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  async function execute(command: Command) {
    if (running.current || !current() || storageBlocked) return
    if (!remember(command)) return
    running.current = true
    setBusy(true)
    setNotice('')
    setRefused(false)
    try {
      if (command.kind === 'save') {
        await saveNativePlan(scope, command.id, command.request)
      } else await archiveNativePlan(scope, command.plan, command.key)
      if (current()) {
        remember(null)
        setOpen(false)
        setArchive(null)
        await load()
      }
    } catch (error) {
      if (current()) {
        const denied = error instanceof CommerceFailure && !error.uncertain
        setRefused(denied)
        setNotice(
          t(
            denied
              ? 'Request rejected. Refresh and review before trying again.'
              : 'Result unknown. Retry the saved request before starting another.'
          )
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  function edit(plan: NativePlan | null) {
    setEditing(plan)
    setTerms(plan?.terms ?? initial)
    setArchive(null)
    setNotice('')
    setOpen(true)
  }
  function submit(event: FormEvent) {
    event.preventDefault()
    if (pending) {
      void execute(pending)
      return
    }
    if (archive) {
      void execute({ kind: 'archive', plan: archive, key: requestKey() })
      return
    }
    const checked = planTermsSchema.safeParse(terms)
    if (!checked.success) {
      setNotice(t('Invalid input'))
      return
    }
    void execute({
      kind: 'save',
      id: editing?.id ?? null,
      request: {
        operation_key: requestKey(),
        expected_revision: editing?.revision ?? null,
        terms: checked.data,
      },
    })
  }
  const locked = busy || !!pending
  const field = <K extends keyof NativePlanTerms>(
    key: K,
    value: NativePlanTerms[K]
  ) => setTerms((old) => ({ ...old, [key]: value }))
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Subscriptions')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <Button
          disabled={busy || !!pending || storageBlocked}
          onClick={() => edit(null)}
        >
          {t('Create Plan')}
        </Button>
        <Button variant='outline' disabled={busy} onClick={() => void load()}>
          {t('Refresh')}
        </Button>
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        {notice && (
          <Alert role='alert'>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        )}
        {storageBlocked && (
          <Alert role='alert'>
            <AlertDescription>
              {t(
                'Recovery storage is unavailable. No new request can be sent.'
              )}
            </AlertDescription>
          </Alert>
        )}
        <ul className='divide-y'>
          {plans.map((plan) => (
            <li
              key={plan.id}
              className='flex flex-wrap items-center justify-between gap-3 py-4'
            >
              <div className='space-y-1'>
                <p className='font-medium'>
                  {plan.terms.title}{' '}
                  <Badge variant='secondary'>
                    {t(
                      plan.archived
                        ? 'Archived'
                        : plan.terms.enabled
                          ? 'Enabled'
                          : 'Disabled'
                    )}
                  </Badge>
                </p>
                <p className='text-muted-foreground text-sm'>
                  {formatCredits(plan.terms.price_units, i18n.language)}{' '}
                  {t('Core credits')} · {t('Quota')}:{' '}
                  {formatCredits(plan.terms.limit_units, i18n.language)} ·{' '}
                  {t('Revision')}: {plan.revision}
                </p>
              </div>
              <div className='flex gap-2'>
                <Button
                  variant='outline'
                  disabled={busy || !!pending || storageBlocked}
                  onClick={() => edit(plan)}
                >
                  {t(plan.archived ? 'Restore' : 'Edit')}
                </Button>
                <Button
                  variant='outline'
                  disabled={
                    busy || !!pending || plan.archived || storageBlocked
                  }
                  onClick={() => {
                    setArchive(plan)
                    setOpen(true)
                    setNotice('')
                  }}
                >
                  {t('Archive')}
                </Button>
              </div>
            </li>
          ))}
        </ul>
        {!plans.length && (
          <p className='text-muted-foreground py-6'>
            {t('No subscription plans yet')}
          </p>
        )}
      </SectionPageLayout.Content>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next && !busy && (!pending || refused)) {
            if (refused) remember(null)
            setOpen(false)
          }
        }}
      >
        <DialogContent className='max-h-[90dvh] overflow-y-auto sm:max-w-xl'>
          <DialogHeader>
            <DialogTitle>
              {t(
                archive ? 'Archive plan' : editing ? 'Edit Plan' : 'Create Plan'
              )}
            </DialogTitle>
            <DialogDescription>
              {t(
                archive
                  ? 'Existing paid benefits and financial history are retained.'
                  : 'Prices use exact credit units. Saved changes apply to new purchases.'
              )}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={submit} className='space-y-4'>
            {notice && (
              <Alert role='alert'>
                <AlertDescription>{notice}</AlertDescription>
              </Alert>
            )}
            {archive ? (
              <p>{archive.terms.title}</p>
            ) : (
              <div className='grid gap-4 sm:grid-cols-2'>
                <div className='space-y-2 sm:col-span-2'>
                  <Label htmlFor='native-plan-title'>{t('Plan Name')}</Label>
                  <Input
                    id='native-plan-title'
                    value={terms.title}
                    disabled={locked}
                    onChange={(e) => field('title', e.target.value)}
                    required
                    maxLength={160}
                  />
                </div>
                {(['price_units', 'limit_units'] as const).map((key) => (
                  <div key={key} className='space-y-2'>
                    <Label htmlFor={`native-plan-${key}`}>
                      {t(
                        key === 'price_units'
                          ? 'Price (credits)'
                          : 'Quota (credits)'
                      )}
                    </Label>
                    <Input
                      id={`native-plan-${key}`}
                      value={terms[key]}
                      disabled={locked}
                      inputMode='numeric'
                      pattern='[1-9][0-9]*'
                      required
                      onChange={(e) => field(key, e.target.value)}
                    />
                  </div>
                ))}
                <div className='space-y-2'>
                  <Label htmlFor='native-plan-duration'>
                    {t('Duration (seconds)')}
                  </Label>
                  <Input
                    id='native-plan-duration'
                    type='number'
                    min={60}
                    max={315576000}
                    value={terms.duration_seconds}
                    disabled={locked}
                    onChange={(e) =>
                      field('duration_seconds', Number(e.target.value))
                    }
                    required
                  />
                </div>
                <div className='space-y-2'>
                  <Label htmlFor='native-plan-limit'>
                    {t('Purchase limit per user')}
                  </Label>
                  <Input
                    id='native-plan-limit'
                    type='number'
                    min={0}
                    max={10000}
                    value={terms.max_purchases_per_user}
                    disabled={locked}
                    onChange={(e) =>
                      field('max_purchases_per_user', Number(e.target.value))
                    }
                  />
                  <p className='text-muted-foreground text-xs'>
                    {t('0 = unlimited')}
                  </p>
                </div>
                <div className='space-y-2'>
                  <Label htmlFor='native-plan-reset'>
                    {t('Quota reset period')}
                  </Label>
                  <NativeSelect
                    id='native-plan-reset'
                    value={terms.reset_period}
                    disabled={locked}
                    onChange={(e) => {
                      field(
                        'reset_period',
                        e.target.value as NativePlanTerms['reset_period']
                      )
                      field(
                        'reset_seconds',
                        e.target.value === 'custom' ? 86400 : 0
                      )
                    }}
                  >
                    {(
                      [
                        'grant',
                        'day',
                        'week',
                        'month',
                        'anniversary_month',
                        'custom',
                      ] as const
                    ).map((period) => (
                      <NativeSelectOption key={period} value={period}>
                        {t(
                          {
                            grant: 'Never',
                            day: 'Daily',
                            week: 'Weekly',
                            month: 'Monthly',
                            anniversary_month: 'Anniversary month',
                            custom: 'Custom',
                          }[period]
                        )}
                      </NativeSelectOption>
                    ))}
                  </NativeSelect>
                </div>
                {terms.reset_period === 'custom' && (
                  <div className='space-y-2'>
                    <Label htmlFor='native-plan-reset-seconds'>
                      {t('Reset interval (seconds)')}
                    </Label>
                    <Input
                      id='native-plan-reset-seconds'
                      type='number'
                      min={1}
                      value={terms.reset_seconds}
                      disabled={locked}
                      onChange={(e) =>
                        field('reset_seconds', Number(e.target.value))
                      }
                    />
                  </div>
                )}
                {(['models', 'groups'] as const).map((key) => (
                  <div key={key} className='space-y-2'>
                    <Label htmlFor={`native-plan-${key}`}>
                      {t(key === 'models' ? 'Models' : 'Groups')}
                    </Label>
                    <Input
                      id={`native-plan-${key}`}
                      value={terms[key].join(',')}
                      disabled={locked}
                      onChange={(e) =>
                        field(
                          key,
                          e.target.value.split(',').map((value) => value.trim())
                        )
                      }
                    />
                    <p className='text-muted-foreground text-xs'>
                      {t('Comma separated; * matches all eligible entries.')}
                    </p>
                  </div>
                ))}
                <div className='flex items-center gap-3 sm:col-span-2'>
                  <Switch
                    id='native-plan-enabled'
                    checked={terms.enabled}
                    disabled={locked}
                    onCheckedChange={(value) => field('enabled', value)}
                  />
                  <Label htmlFor='native-plan-enabled'>{t('Enabled')}</Label>
                </div>
              </div>
            )}
            <DialogFooter>
              {(!pending || refused) && (
                <Button
                  type='button'
                  variant='outline'
                  disabled={busy}
                  onClick={() => {
                    if (refused) remember(null)
                    setOpen(false)
                  }}
                >
                  {t('Cancel')}
                </Button>
              )}
              <Button type='submit' disabled={busy || storageBlocked}>
                {t(pending ? 'Retry saved request' : 'Confirm')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </SectionPageLayout>
  )
}
