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
import { Textarea } from '@/components/ui/textarea'
import {
  commerceScope,
  CommerceFailure,
  currentCommerceScope,
  formatCredits,
  nativeToken,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  campaignSchema,
  campaignSpecSchema,
  codeStatusSchema,
  issueCodes,
  issueRequestSchema,
  listCampaigns,
  listCodes,
  revokeCode,
  saveCampaign,
  saveCampaignSchema,
  type CampaignSpec,
  type NativeCampaign,
  type NativeCodeStatus,
  type NativeIssue,
} from './native-api'

const commandSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('save'),
      id: nativeToken.nullable(),
      request: saveCampaignSchema,
    })
    .strict(),
  z
    .object({
      kind: z.literal('issue'),
      campaign: campaignSchema,
      request: issueRequestSchema,
    })
    .strict(),
  z.object({ kind: z.literal('revoke'), code: codeStatusSchema }).strict(),
])
type Command = z.infer<typeof commandSchema>
function initial(): CampaignSpec {
  const now = Math.floor(Date.now() / 1000)
  return {
    kind: 'redemption',
    title: '',
    award_units: '10000',
    budget_units: '1000000',
    max_claims: 100,
    max_claims_per_user: 1,
    starts_at: now,
    ends_at: now + 30 * 86400,
    enabled: false,
  }
}
const utcInput = (time: number) =>
  Number.isFinite(time) ? new Date(time * 1000).toISOString().slice(0, 16) : ''
const utcTime = (value: string) =>
  Math.floor(new Date(`${value}:00Z`).getTime() / 1000)
export function NativePromotionAdmin() {
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
    <CampaignAdmin
      key={`${auth.user?.id}:${auth.session?.sid}`}
      scope={scope}
    />
  )
}
function CampaignAdmin({ scope }: { scope: CommerceScope }) {
  const { t, i18n } = useTranslation()
  const [campaigns, setCampaigns] = useState<NativeCampaign[]>([])
  const [editing, setEditing] = useState<NativeCampaign | null>(null)
  const [spec, setSpec] = useState<CampaignSpec>(initial)
  const [mode, setMode] = useState<'save' | 'issue' | 'codes' | 'revoke'>(
    'save'
  )
  const [quantity, setQuantity] = useState(1)
  const [codes, setCodes] = useState<NativeCodeStatus[]>([])
  const [selectedCode, setSelectedCode] = useState<NativeCodeStatus | null>(
    null
  )
  const [issued, setIssued] = useState<NativeIssue | null>(null)
  const [open, setOpen] = useState(false)
  const [pending, setPending] = useState<Command | null>(null)
  const [busy, setBusy] = useState(false)
  const [blocked, setBlocked] = useState(false)
  const [notice, setNotice] = useState('')
  const [resolved, setResolved] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const current = () => mounted.current && currentCommerceScope(scope)
  async function load() {
    try {
      const result = await listCampaigns(scope, true)
      if (current()) setCampaigns(result)
    } catch {
      if (current()) setNotice(t('Request failed'))
    }
  }
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(scope, 'promotion-admin', commandSchema)
      setPending(saved)
      if (saved) {
        setOpen(true)
        setMode(saved.kind)
        if (saved.kind === 'save') setSpec(saved.request.spec)
        if (saved.kind === 'issue') {
          setEditing(saved.campaign)
          setQuantity(saved.request.quantity)
        }
        if (saved.kind === 'revoke') setSelectedCode(saved.code)
      }
    } catch {
      setBlocked(true)
    }
    void load()
    return () => {
      mounted.current = false
    }
    // Account/session changes remount this workspace.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    if (!issued) return
    const clear = () => {
      if (document.hidden) setIssued(null)
    }
    const timer = window.setTimeout(() => setIssued(null), 5 * 60 * 1000)
    document.addEventListener('visibilitychange', clear)
    return () => {
      window.clearTimeout(timer)
      document.removeEventListener('visibilitychange', clear)
    }
  }, [issued])
  function remember(value: Command | null) {
    try {
      savePending(scope, 'promotion-admin', value)
      setPending(value)
      return true
    } catch {
      setBlocked(true)
      return false
    }
  }
  async function execute(command: Command) {
    if (running.current || blocked || !current() || !remember(command)) return
    running.current = true
    setBusy(true)
    setNotice('')
    setResolved(false)
    try {
      if (command.kind === 'issue') {
        const result = await issueCodes(
          scope,
          command.campaign,
          command.request
        )
        if (current()) {
          setIssued(result)
          setResolved(true)
        }
      } else {
        if (command.kind === 'save') {
          await saveCampaign(scope, command.id, command.request)
        } else await revokeCode(scope, command.code)
        if (current()) {
          remember(null)
          setOpen(false)
          setResolved(true)
        }
      }
      if (current()) await load()
    } catch (error) {
      if (current()) {
        const denied = error instanceof CommerceFailure && !error.uncertain
        setResolved(denied)
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
  function edit(campaign: NativeCampaign | null) {
    setEditing(campaign)
    setSpec(campaign?.spec ?? initial())
    setMode('save')
    setNotice('')
    setOpen(true)
  }
  async function showCodes(campaign: NativeCampaign) {
    setEditing(campaign)
    setMode('codes')
    setOpen(true)
    setNotice('')
    setCodes([])
    try {
      const result = await listCodes(scope, campaign)
      if (current()) setCodes(result)
    } catch {
      if (current()) setNotice(t('Request failed'))
    }
  }
  function submit(event: FormEvent) {
    event.preventDefault()
    if (pending) {
      void execute(pending)
      return
    }
    if (mode === 'issue' && editing) {
      const request = issueRequestSchema.safeParse({
        operation_key: requestKey(),
        quantity,
      })
      if (!request.success) {
        setNotice(t('Invalid input'))
        return
      }
      void execute({ kind: 'issue', campaign: editing, request: request.data })
      return
    }
    if (mode === 'revoke' && selectedCode) {
      void execute({ kind: 'revoke', code: selectedCode })
      return
    }
    const parsed = campaignSpecSchema.safeParse(spec)
    if (
      !parsed.success ||
      parsed.data.ends_at <= parsed.data.starts_at ||
      BigInt(parsed.data.award_units) > BigInt(parsed.data.budget_units)
    ) {
      setNotice(t('Invalid input'))
      return
    }
    void execute({
      kind: 'save',
      id: editing?.id ?? null,
      request: {
        operation_key: requestKey(),
        expected_revision: editing?.revision ?? null,
        spec: parsed.data,
      },
    })
  }
  function close() {
    if (busy || (pending && !resolved)) return
    if (pending) remember(null)
    setIssued(null)
    setOpen(false)
    setResolved(false)
  }
  const field = <K extends keyof CampaignSpec>(
    key: K,
    value: CampaignSpec[K]
  ) => setSpec((old) => ({ ...old, [key]: value }))
  const credits = (value: string) =>
    `${formatCredits(value, i18n.language)} ${t('Core credits')}`
  const locked = busy || !!pending
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Rewards')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <Button
          disabled={busy || !!pending || blocked}
          onClick={() => edit(null)}
        >
          {t('Create campaign')}
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
        {blocked && (
          <Alert role='alert'>
            <AlertDescription>
              {t(
                'Recovery storage is unavailable. No new request can be sent.'
              )}
            </AlertDescription>
          </Alert>
        )}
        <p className='text-muted-foreground mb-4 text-sm'>
          {t(
            'Issued codes reserve campaign budget. Unclaimed revoked codes release that reservation.'
          )}
        </p>
        <ul className='divide-y'>
          {campaigns.map((campaign) => (
            <li
              key={campaign.id}
              className='flex flex-wrap items-center justify-between gap-3 py-4'
            >
              <div className='space-y-1'>
                <p className='font-medium'>
                  {campaign.spec.title}{' '}
                  <Badge variant='secondary'>
                    {t(campaign.spec.enabled ? 'Enabled' : 'Disabled')}
                  </Badge>
                </p>
                <p className='text-sm'>
                  {t('Reward')}: {credits(campaign.spec.award_units)} ·{' '}
                  {t('Budget')}: {credits(campaign.spec.budget_units)}
                </p>
                <p className='text-muted-foreground text-sm'>
                  {t('Used')}: {credits(campaign.spent_units)} · {t('Reserved')}
                  : {credits(campaign.reserved_units)}
                </p>
              </div>
              <div className='flex flex-wrap gap-2'>
                <Button
                  variant='outline'
                  disabled={busy || !!pending || blocked}
                  onClick={() => edit(campaign)}
                >
                  {t('Edit')}
                </Button>
                {campaign.spec.kind === 'redemption' && (
                  <>
                    <Button
                      variant='outline'
                      disabled={
                        busy || !!pending || blocked || !campaign.spec.enabled
                      }
                      onClick={() => {
                        setEditing(campaign)
                        setMode('issue')
                        setQuantity(1)
                        setIssued(null)
                        setResolved(false)
                        setNotice('')
                        setOpen(true)
                      }}
                    >
                      {t('Generate codes')}
                    </Button>
                    <Button
                      variant='outline'
                      disabled={busy || !!pending}
                      onClick={() => void showCodes(campaign)}
                    >
                      {t('Codes')}
                    </Button>
                  </>
                )}
              </div>
            </li>
          ))}
        </ul>
      </SectionPageLayout.Content>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!value) close()
        }}
      >
        <DialogContent className='max-h-[90dvh] overflow-y-auto sm:max-w-xl'>
          <DialogHeader>
            <DialogTitle>
              {t(
                mode === 'save'
                  ? editing
                    ? 'Edit campaign'
                    : 'Create campaign'
                  : mode === 'issue'
                    ? 'Generate codes'
                    : mode === 'revoke'
                      ? 'Revoke code'
                      : 'Redemption Codes'
              )}
            </DialogTitle>
            <DialogDescription>
              {t(
                mode === 'save'
                  ? 'Rewards and budgets use exact credits. All campaign times are UTC.'
                  : mode === 'issue'
                    ? 'Copy issued codes before closing. Codes clear when hidden or after five minutes.'
                    : 'Claimed rewards remain in the financial history.'
              )}
            </DialogDescription>
          </DialogHeader>
          {notice && (
            <Alert role='alert'>
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          {mode === 'codes' ? (
            <>
              <ul className='divide-y'>
                {codes.map((code) => (
                  <li
                    key={code.id}
                    className='flex items-center justify-between gap-2 py-2'
                  >
                    <span className='min-w-0 truncate font-mono text-xs'>
                      {code.id}
                    </span>
                    <Badge variant='secondary'>
                      {t(
                        code.claimed
                          ? 'Claimed'
                          : code.revoked
                            ? 'Revoked'
                            : 'Available'
                      )}
                    </Badge>
                    <Button
                      size='sm'
                      variant='outline'
                      disabled={code.claimed || code.revoked}
                      onClick={() => {
                        setSelectedCode(code)
                        setMode('revoke')
                      }}
                    >
                      {t('Revoke')}
                    </Button>
                  </li>
                ))}
              </ul>
              <DialogFooter>
                <Button onClick={close}>{t('Close')}</Button>
              </DialogFooter>
            </>
          ) : (
            <form onSubmit={submit} className='space-y-4'>
              {mode === 'save' && (
                <div className='grid gap-4 sm:grid-cols-2'>
                  <div className='space-y-2 sm:col-span-2'>
                    <Label htmlFor='campaign-title'>{t('Name')}</Label>
                    <Input
                      id='campaign-title'
                      value={spec.title}
                      disabled={locked}
                      required
                      maxLength={160}
                      onChange={(e) => field('title', e.target.value)}
                    />
                  </div>
                  <div className='space-y-2'>
                    <Label htmlFor='campaign-kind'>{t('Type')}</Label>
                    <NativeSelect
                      id='campaign-kind'
                      value={spec.kind}
                      disabled={locked || !!editing}
                      onChange={(e) => {
                        field('kind', e.target.value as CampaignSpec['kind'])
                        if (e.target.value === 'envelope') {
                          field('max_claims_per_user', 1)
                        }
                      }}
                    >
                      <NativeSelectOption value='redemption'>
                        {t('Redemption Codes')}
                      </NativeSelectOption>
                      <NativeSelectOption value='checkin'>
                        {t('Daily check-in')}
                      </NativeSelectOption>
                      <NativeSelectOption value='envelope'>
                        {t('Activity reward')}
                      </NativeSelectOption>
                    </NativeSelect>
                  </div>
                  {(['award_units', 'budget_units'] as const).map((key) => (
                    <div key={key} className='space-y-2'>
                      <Label htmlFor={`campaign-${key}`}>
                        {t(
                          key === 'award_units'
                            ? 'Reward (credits)'
                            : 'Budget (credits)'
                        )}
                      </Label>
                      <Input
                        id={`campaign-${key}`}
                        value={spec[key]}
                        disabled={
                          locked || (key === 'award_units' && !!editing)
                        }
                        inputMode='numeric'
                        pattern='[1-9][0-9]*'
                        required
                        onChange={(e) => field(key, e.target.value)}
                      />
                    </div>
                  ))}
                  {(['max_claims', 'max_claims_per_user'] as const).map(
                    (key) => (
                      <div key={key} className='space-y-2'>
                        <Label htmlFor={`campaign-${key}`}>
                          {t(
                            key === 'max_claims'
                              ? 'Maximum claims'
                              : 'Claims per user'
                          )}
                        </Label>
                        <Input
                          id={`campaign-${key}`}
                          type='number'
                          min={1}
                          max={key === 'max_claims' ? 1000000 : 10000}
                          value={spec[key]}
                          disabled={
                            locked ||
                            (key === 'max_claims_per_user' &&
                              spec.kind === 'envelope')
                          }
                          required
                          onChange={(e) => field(key, Number(e.target.value))}
                        />
                      </div>
                    )
                  )}
                  {(['starts_at', 'ends_at'] as const).map((key) => (
                    <div key={key} className='space-y-2'>
                      <Label htmlFor={`campaign-${key}`}>
                        {t(
                          key === 'starts_at'
                            ? 'Starts at (UTC)'
                            : 'Ends at (UTC)'
                        )}
                      </Label>
                      <Input
                        id={`campaign-${key}`}
                        type='datetime-local'
                        value={utcInput(spec[key])}
                        disabled={locked}
                        required
                        onChange={(e) => field(key, utcTime(e.target.value))}
                      />
                    </div>
                  ))}
                  <div className='flex items-center gap-3 sm:col-span-2'>
                    <Switch
                      id='campaign-enabled'
                      checked={spec.enabled}
                      disabled={locked}
                      onCheckedChange={(value) => field('enabled', value)}
                    />
                    <Label htmlFor='campaign-enabled'>{t('Enabled')}</Label>
                  </div>
                </div>
              )}
              {mode === 'issue' && (
                <>
                  <div className='space-y-2'>
                    <Label htmlFor='campaign-quantity'>{t('Quantity')}</Label>
                    <Input
                      id='campaign-quantity'
                      type='number'
                      min={1}
                      max={100}
                      value={quantity}
                      disabled={locked}
                      onChange={(e) => setQuantity(Number(e.target.value))}
                      required
                    />
                  </div>
                  {editing &&
                    quantity >= 1 &&
                    quantity <= 100 &&
                    Number.isSafeInteger(quantity) && (
                      <p className='text-sm'>
                        {t('Reserved')}:{' '}
                        {credits(
                          (
                            BigInt(editing.spec.award_units) * BigInt(quantity)
                          ).toString()
                        )}
                      </p>
                    )}
                  {issued && (
                    <div className='space-y-2'>
                      <Label htmlFor='campaign-issued-codes'>
                        {t('Issued redemption codes')}
                      </Label>
                      <Textarea
                        id='campaign-issued-codes'
                        readOnly
                        value={issued.codes.map((code) => code.code).join('\n')}
                        rows={Math.min(6, issued.codes.length)}
                      />
                      <Button
                        type='button'
                        variant='outline'
                        onClick={() =>
                          void navigator.clipboard
                            .writeText(
                              issued.codes.map((code) => code.code).join('\n')
                            )
                            .then(() => {
                              if (current()) setNotice(t('Copied!'))
                            })
                            .catch(() => {
                              if (current()) setNotice(t('Request failed'))
                            })
                        }
                      >
                        {t('Copy')}
                      </Button>
                    </div>
                  )}
                </>
              )}
              {mode === 'revoke' && (
                <p className='font-mono text-sm break-all'>
                  {selectedCode?.id}
                </p>
              )}
              <DialogFooter>
                {(!pending || resolved) && (
                  <Button
                    type='button'
                    variant='outline'
                    disabled={busy}
                    onClick={close}
                  >
                    {t('Close')}
                  </Button>
                )}
                <Button type='submit' disabled={busy || blocked}>
                  {t(pending ? 'Retry saved request' : 'Confirm')}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </SectionPageLayout>
  )
}
