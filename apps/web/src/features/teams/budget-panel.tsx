/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Pencil, Plus, Wallet } from 'lucide-react'
import { useId, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

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
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  currentTeamScope,
  TeamFailure,
  type Member,
  type Team,
  type TeamScope,
} from './api'
import {
  budgetProgress,
  listBudgets,
  saveBudget,
  validCreditLimit,
  type Budget,
  type BudgetDraft,
} from './budget-api'

const selectClass =
  'border-input bg-background min-h-11 w-full min-w-0 rounded-md border px-3 py-2 text-sm'
type Editor = {
  current?: Budget
  target: string
  period: Budget['period']
  limit: string
}

export function PersonalBudgetPanel() {
  const auth = useAuthStore((state) => state.auth)
  if (!auth.user || !auth.session?.sid) return null
  const scope = { userId: auth.user.id, sessionId: auth.session.sid }
  return (
    <SpendingBudgets key={`${scope.userId}:${scope.sessionId}`} scope={scope} />
  )
}

export function TeamBudgetPanel({
  scope,
  team,
  members,
}: {
  scope: TeamScope
  team: Team
  members: Member[]
}) {
  return (
    <SpendingBudgets
      key={`${scope.userId}:${scope.sessionId}:${team.id}`}
      scope={scope}
      team={team}
      members={members}
    />
  )
}

function SpendingBudgets({
  scope,
  team,
  members = [],
}: {
  scope: TeamScope
  team?: Team
  members?: Member[]
}) {
  const { t, i18n } = useTranslation()
  const id = useId()
  const client = useQueryClient()
  const queryKey = [
    'native-budgets',
    scope.userId,
    scope.sessionId,
    team?.id ?? 'self',
  ] as const
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => listBudgets(scope, team?.id, signal),
    retry: false,
  })
  const [editor, setEditor] = useState<Editor>()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const saving = useRef(false)
  const editableMembers = members.filter(
    (member) =>
      team?.role === 'owner' ||
      member.role === 'member' ||
      member.user_id === scope.userId
  )
  const manager = !team || team.role !== 'member'
  const canCreate =
    manager && (!team || team.role === 'owner' || editableMembers.length > 0)
  const number = (value: string) =>
    new Intl.NumberFormat(
      toIntlLocale(i18n.resolvedLanguage ?? i18n.language)
    ).format(BigInt(value))
  const memberName = (userId: number) => {
    const member = members.find((item) => item.user_id === userId)
    return (
      member?.display_name ||
      member?.username ||
      t('User ID {{id}}', { id: userId })
    )
  }
  const title = (budget: Budget) =>
    budget.scope === 'self'
      ? t('Personal budget')
      : budget.scope === 'account'
        ? t('Team budget')
        : budget.user_id === null
          ? t('Member')
          : memberName(budget.user_id)
  const targetOf = (budget: Budget) =>
    budget.scope === 'member' ? String(budget.user_id) : budget.scope
  const duplicate =
    editor &&
    !editor.current &&
    query.data?.items.find(
      (budget) =>
        budget.issuer_user_id === scope.userId &&
        targetOf(budget) === editor.target &&
        budget.period === editor.period
    )
  function openEditor(budget?: Budget) {
    setError('')
    setEditor({
      current: budget,
      target: budget
        ? targetOf(budget)
        : !team
          ? 'self'
          : team.role === 'owner'
            ? 'account'
            : String(editableMembers[0]?.user_id ?? ''),
      period: budget?.period ?? 'month',
      limit: budget?.limit_credits ?? '',
    })
  }
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (
      !editor ||
      saving.current ||
      !currentTeamScope(scope) ||
      !validCreditLimit(editor.limit) ||
      duplicate
    ) {
      return
    }
    saving.current = true
    setBusy(true)
    setError('')
    const draft: BudgetDraft = {
      scope: !team
        ? 'self'
        : editor.target === 'account'
          ? 'account'
          : 'member',
      ...(!['self', 'account'].includes(editor.target)
        ? { user_id: Number(editor.target) }
        : {}),
      period: editor.period,
      limit_credits: editor.limit.trim(),
      expected_revision: editor.current?.revision ?? null,
    }
    try {
      await saveBudget(scope, draft, team?.id)
      if (!currentTeamScope(scope)) return
      setEditor(undefined)
      toast.success(t('Update succeeded'))
      await client.invalidateQueries({ queryKey })
    } catch (cause) {
      if (!currentTeamScope(scope)) return
      setError(
        t(
          cause instanceof TeamFailure && cause.kind === 'conflict'
            ? 'This budget changed. Reload it before saving again.'
            : cause instanceof TeamFailure && cause.kind === 'unknown'
              ? 'Save result is unknown. Check the original operation before continuing.'
              : 'Request failed'
        )
      )
      await client.invalidateQueries({ queryKey })
    } finally {
      saving.current = false
      setBusy(false)
    }
  }
  return (
    <section
      className='min-w-0 space-y-5'
      aria-label={t(team ? 'Team budget' : 'Personal budget')}
    >
      <header className='flex flex-wrap items-start justify-between gap-4'>
        <div className='max-w-2xl space-y-2'>
          <h3 className='text-base font-semibold'>
            {t(team ? 'Spending limits' : 'Personal budget')}
          </h3>
          <p className='text-muted-foreground text-sm leading-relaxed'>
            {t(
              'Budget limits new spending without changing the account balance.'
            )}
          </p>
        </div>
        {canCreate && (
          <Button
            variant='outline'
            disabled={query.isPending || query.isError || busy}
            onClick={() => openEditor()}
          >
            <Plus className='size-4' />
            {t('Add spending limit')}
          </Button>
        )}
      </header>
      {query.isPending ? (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Loading')}
        </p>
      ) : query.isError ? (
        <Alert role='alert'>
          <AlertDescription className='flex flex-wrap items-center justify-between gap-3'>
            <span>{t('Could not load spending limits')}</span>
            <Button
              variant='outline'
              size='sm'
              onClick={() => void query.refetch()}
            >
              {t('Retry')}
            </Button>
          </AlertDescription>
        </Alert>
      ) : query.data?.items.length === 0 ? (
        <div className='bg-muted/40 flex items-start gap-4 rounded-xl p-5'>
          <Wallet className='text-muted-foreground mt-1 size-5 shrink-0' />
          <div className='space-y-1'>
            <p className='text-sm font-medium'>{t('No spending limits')}</p>
            <p className='text-muted-foreground text-sm'>
              {t('Available funds and other applicable limits still apply.')}
            </p>
          </div>
        </div>
      ) : (
        <div className='grid gap-4 xl:grid-cols-2'>
          {query.data?.items.map((budget) => (
            <article
              key={budget.id}
              className='bg-card min-w-0 space-y-5 rounded-xl p-5 sm:p-6'
            >
              <header className='flex items-start justify-between gap-3'>
                <div className='min-w-0 space-y-1.5'>
                  <div className='flex flex-wrap items-center gap-2'>
                    <h4 className='truncate font-medium'>{title(budget)}</h4>
                    <Badge variant='secondary'>
                      {t(budget.period === 'day' ? 'Daily' : 'Monthly')}
                    </Badge>
                  </div>
                  <p className='text-muted-foreground text-xs'>
                    {t('Set by user {{id}}', { id: budget.issuer_user_id })}
                  </p>
                </div>
                {budget.can_edit && (
                  <Button
                    variant='ghost'
                    size='sm'
                    onClick={() => openEditor(budget)}
                  >
                    <Pencil className='size-3.5' />
                    {t('Edit')}
                  </Button>
                )}
              </header>
              <div>
                <p className='text-muted-foreground text-xs'>
                  {t('Remaining')}
                </p>
                <p className='mt-1 text-2xl font-semibold tracking-tight break-words tabular-nums'>
                  {number(budget.remaining_credits)}{' '}
                  <span className='text-muted-foreground text-xs font-normal'>
                    credit
                  </span>
                </p>
                <div
                  role='progressbar'
                  aria-label={t('Budget usage')}
                  aria-valuenow={budgetProgress(budget)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  className='bg-muted mt-4 h-1.5 overflow-hidden rounded-full'
                >
                  <div
                    className={`h-full rounded-full ${budget.remaining_credits === '0' ? 'bg-destructive' : 'bg-primary'}`}
                    style={{ width: `${budgetProgress(budget)}%` }}
                  />
                </div>
              </div>
              <dl className='grid grid-cols-3 gap-3 text-sm'>
                {(
                  [
                    ['Limit', budget.limit_credits],
                    ['Used', budget.consumed_credits],
                    ['Reserved', budget.reserved_credits],
                  ] as const
                ).map(([label, amount]) => (
                  <div key={label} className='min-w-0'>
                    <dt className='text-muted-foreground text-xs'>
                      {t(label)}
                    </dt>
                    <dd className='mt-1 font-medium break-words tabular-nums'>
                      {number(amount)}
                    </dd>
                  </div>
                ))}
              </dl>
              <p className='text-muted-foreground text-xs leading-relaxed'>
                {t('Resets at {{time}} UTC', {
                  time: new Intl.DateTimeFormat(
                    toIntlLocale(i18n.resolvedLanguage ?? i18n.language),
                    { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' }
                  ).format(budget.window_end * 1000),
                })}
              </p>
            </article>
          ))}
        </div>
      )}
      <p className='text-muted-foreground text-xs leading-relaxed'>
        {t(
          'All limits apply together. Members cannot raise limits set by team managers.'
        )}
      </p>
      <Dialog
        open={Boolean(editor)}
        onOpenChange={(open) => {
          if (!open && !busy) {
            setEditor(undefined)
            setError('')
          }
        }}
      >
        <DialogContent className='sm:max-w-lg'>
          <DialogHeader>
            <DialogTitle>
              {t(
                editor?.current ? 'Edit spending limit' : 'Add spending limit'
              )}
            </DialogTitle>
            <DialogDescription>
              {t(
                'A limit of 0 blocks new spending. Daily and monthly limits reset at 00:00 UTC.'
              )}
            </DialogDescription>
          </DialogHeader>
          {editor && (
            <form className='space-y-5' onSubmit={submit}>
              {team && (
                <div className='space-y-2'>
                  <Label htmlFor={`${id}-target`}>
                    {t('Budget applies to')}
                  </Label>
                  <select
                    id={`${id}-target`}
                    className={selectClass}
                    value={editor.target}
                    disabled={busy || Boolean(editor.current)}
                    onChange={(event) =>
                      setEditor({ ...editor, target: event.target.value })
                    }
                  >
                    {(team.role === 'owner' || editor.target === 'account') && (
                      <option value='account'>{t('Whole team')}</option>
                    )}
                    {editableMembers.map((member) => (
                      <option key={member.user_id} value={member.user_id}>
                        {memberName(member.user_id)}
                      </option>
                    ))}
                  </select>
                </div>
              )}
              <div className='space-y-2'>
                <Label htmlFor={`${id}-period`}>{t('Period')}</Label>
                <select
                  id={`${id}-period`}
                  className={selectClass}
                  value={editor.period}
                  disabled={busy || Boolean(editor.current)}
                  onChange={(event) =>
                    setEditor({
                      ...editor,
                      period: event.target.value as Budget['period'],
                    })
                  }
                >
                  <option value='day'>{t('Daily')}</option>
                  <option value='month'>{t('Monthly')}</option>
                </select>
              </div>
              <div className='space-y-2'>
                <Label htmlFor={`${id}-limit`}>{t('Limit (credits)')}</Label>
                <Input
                  id={`${id}-limit`}
                  value={editor.limit}
                  onChange={(event) =>
                    setEditor({ ...editor, limit: event.target.value })
                  }
                  inputMode='numeric'
                  pattern='[0-9]+'
                  maxLength={19}
                  required
                  disabled={busy}
                  autoComplete='off'
                />
              </div>
              {duplicate && (
                <Alert role='alert'>
                  <AlertDescription className='space-y-3'>
                    <p>
                      {t('A limit already exists for this scope and period.')}
                    </p>
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      onClick={() => openEditor(duplicate)}
                    >
                      {t('Edit existing limit')}
                    </Button>
                  </AlertDescription>
                </Alert>
              )}
              {error && (
                <Alert role='alert'>
                  <AlertDescription className='space-y-3'>
                    <p>{error}</p>
                    <Button
                      type='button'
                      variant='outline'
                      size='sm'
                      disabled={busy}
                      onClick={async () => {
                        const result = await query.refetch()
                        const fresh = result.data?.items.find(
                          (item) => item.id === editor.current?.id
                        )
                        if (fresh && fresh.can_edit) openEditor(fresh)
                        else {
                          setEditor(undefined)
                          setError('')
                        }
                      }}
                    >
                      {t('Reload')}
                    </Button>
                  </AlertDescription>
                </Alert>
              )}
              <DialogFooter>
                <Button
                  type='button'
                  variant='outline'
                  disabled={busy}
                  onClick={() => setEditor(undefined)}
                >
                  {t('Cancel')}
                </Button>
                <Button
                  type='submit'
                  disabled={
                    busy ||
                    !validCreditLimit(editor.limit) ||
                    Boolean(duplicate) ||
                    Boolean(error)
                  }
                >
                  {t(busy ? 'Saving...' : 'Save')}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </section>
  )
}
