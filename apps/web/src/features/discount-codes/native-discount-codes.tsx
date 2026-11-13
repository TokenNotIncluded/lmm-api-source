/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import {
  currentCommerceScope,
  readPending,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  discountBatchSchema,
  generateDiscountBatch,
  getDiscountPage,
  removeDiscount,
  removeExhaustedDiscounts,
  saveDiscount,
  setDiscountEnabled,
  type DiscountBatch,
  type DiscountFilter,
  type DiscountPage,
  type NativeDiscount,
} from './native-api'
import {
  discountDefinition,
  discountTerms,
  editDiscountForm,
  emptyDiscountForm,
  type DiscountForm,
} from './native-form'
import { buildDiscountCodeLink } from './share-link'

type Editor =
  | { kind: 'closed' }
  | { kind: 'create' }
  | { kind: 'batch' }
  | { kind: 'edit'; code: NativeDiscount }
  | { kind: 'delete'; code: NativeDiscount }
  | { kind: 'cleanup' }
const pendingArea = 'discount-code-batch'
function restoreBatch(scope: CommerceScope) {
  try {
    return {
      request: readPending(scope, pendingArea, discountBatchSchema),
      failed: false,
    }
  } catch {
    return { request: null, failed: true }
  }
}
export function NativeDiscountCodes() {
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const scope = useMemo(
    () => (userId && sessionId ? { userId, sessionId } : undefined),
    [userId, sessionId]
  )
  return scope ? <Codes key={`${userId}:${sessionId}`} scope={scope} /> : null
}
function Codes({ scope }: { scope: CommerceScope }) {
  const { t, i18n } = useTranslation()
  const [page, setPage] = useState<DiscountPage | null>(null)
  const [after, setAfter] = useState('0')
  const [previous, setPrevious] = useState<string[]>([])
  const [keyword, setKeyword] = useState('')
  const [status, setStatus] = useState<DiscountFilter['enabled']>('all')
  const [filter, setFilter] = useState<DiscountFilter>({
    keyword: '',
    enabled: 'all',
  })
  const [refresh, setRefresh] = useState(0)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const running = useRef(false)
  const alive = useRef(true)
  const [error, setError] = useState<
    'read' | 'write' | 'fields' | 'copy' | null
  >(null)
  const [editor, setEditor] = useState<Editor>({ kind: 'closed' })
  const [form, setForm] = useState<DiscountForm>(emptyDiscountForm)
  const [restored] = useState(() => restoreBatch(scope))
  const [pending, setPending] = useState<DiscountBatch | null>(restored.request)
  const [storageFailed, setStorageFailed] = useState(restored.failed)
  const [generated, setGenerated] = useState<NativeDiscount[]>([])
  const current = () => alive.current && currentCommerceScope(scope)
  const reload = () => setRefresh((value) => value + 1)
  const number = (value: string) =>
    BigInt(value).toLocaleString(
      toIntlLocale(i18n.resolvedLanguage || i18n.language)
    )
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])
  useEffect(() => {
    let active = true
    setLoading(true)
    setPage(null)
    void getDiscountPage(scope, after, filter)
      .then((data) => {
        if (active && currentCommerceScope(scope)) {
          setPage(data)
          setError((old) => (old === 'read' ? null : old))
        }
      })
      .catch(() => {
        if (active && currentCommerceScope(scope)) setError('read')
      })
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [scope, after, filter, refresh])
  const run = async (operation: () => Promise<void>) => {
    if (running.current || !current()) return
    running.current = true
    setBusy(true)
    setError(null)
    try {
      await operation()
    } catch {
      if (current()) setError('write')
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  const open = (next: Editor) => {
    try {
      setForm(
        next.kind === 'edit'
          ? editDiscountForm(next.code)
          : { ...emptyDiscountForm }
      )
      setError(null)
      setEditor(next)
    } catch {
      setError('fields')
    }
  }
  const recover = (request: DiscountBatch) =>
    run(async () => {
      const items = await generateDiscountBatch(scope, request)
      if (!current()) return
      savePending(scope, pendingArea, null)
      setPending(null)
      setGenerated(items)
      setEditor({ kind: 'closed' })
      reload()
    })
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (
      editor.kind !== 'edit' &&
      editor.kind !== 'create' &&
      editor.kind !== 'batch'
    ) {
      return
    }
    try {
      if (editor.kind === 'batch') {
        if (pending || storageFailed) return
        const request = discountBatchSchema.parse({
          ...discountTerms(form),
          count: Number(form.count),
          request_key: crypto.randomUUID(),
        })
        try {
          savePending(scope, pendingArea, request)
        } catch {
          setStorageFailed(true)
          return
        }
        setPending(request)
        setEditor({ kind: 'closed' })
        void recover(request)
      } else {
        const input = discountDefinition(form)
        const editing = editor.kind === 'edit' ? editor.code : undefined
        void run(async () => {
          await saveDiscount(scope, input, editing)
          if (current()) {
            setEditor({ kind: 'closed' })
            reload()
          }
        })
      }
    } catch {
      setError('fields')
    }
  }
  const copy = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value)
    } catch {
      if (current()) setError('copy')
    }
  }
  const change = (
    field: keyof Omit<DiscountForm, 'enabled' | 'source'>,
    value: string
  ) => setForm((old) => ({ ...old, [field]: value }))
  const field = (
    name:
      | 'code'
      | 'name'
      | 'percent'
      | 'minimum'
      | 'maximum'
      | 'count'
      | 'starts'
      | 'ends',
    label: string,
    type = 'text'
  ) => (
    <div className='space-y-2' key={name}>
      <Label htmlFor={`native-discount-${name}`}>{label}</Label>
      <Input
        id={`native-discount-${name}`}
        value={form[name]}
        type={type}
        step={type === 'datetime-local' ? 1 : undefined}
        required={name !== 'starts' && name !== 'ends'}
        disabled={busy || Boolean(pending)}
        autoComplete='off'
        onChange={(event) => change(name, event.target.value)}
      />
    </div>
  )
  const message =
    error === 'read'
      ? t('Unable to load discount codes.')
      : error === 'fields'
        ? t('Check the discount code fields.')
        : error === 'copy'
          ? t('Failed to copy')
          : t(
              'Discount code request failed. Refresh the list to check the result before retrying.'
            )
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Discount Codes')}</SectionPageLayout.Title>
      <SectionPageLayout.Actions>
        <Button
          disabled={busy || Boolean(pending) || storageFailed}
          onClick={() => open({ kind: 'create' })}
        >
          {t('Create discount code')}
        </Button>
        <Button
          variant='outline'
          disabled={busy || Boolean(pending) || storageFailed}
          onClick={() => open({ kind: 'batch' })}
        >
          {t('Generate codes')}
        </Button>
        <Button
          variant='outline'
          disabled={busy}
          onClick={() => open({ kind: 'cleanup' })}
        >
          {t('Delete exhausted codes')}
        </Button>
      </SectionPageLayout.Actions>
      <SectionPageLayout.Content>
        <div className='space-y-5'>
          {error && editor.kind === 'closed' ? (
            <p role='alert' className='text-destructive text-sm'>
              {message}
            </p>
          ) : null}
          {storageFailed ? (
            <p role='alert'>
              {t(
                'Saved code generation could not be read. Do not generate another batch until its result is checked.'
              )}
            </p>
          ) : null}
          {pending ? (
            <div className='bg-muted space-y-3 rounded-xl p-4'>
              <p>
                {t(
                  'Pending code generation is saved. Recover the same batch before starting another.'
                )}
              </p>
              <Button disabled={busy} onClick={() => void recover(pending)}>
                {t('Recover generated codes')}
              </Button>
            </div>
          ) : null}
          <form
            className='flex flex-wrap gap-3'
            onSubmit={(event) => {
              event.preventDefault()
              setFilter({ keyword, enabled: status })
              setAfter('0')
              setPrevious([])
            }}
          >
            <Label className='sr-only' htmlFor='native-discount-search'>
              {t('Search')}
            </Label>
            <Input
              id='native-discount-search'
              className='max-w-sm'
              value={keyword}
              onChange={(event) => setKeyword(event.target.value)}
            />
            <NativeSelect
              aria-label={t('Status')}
              value={status}
              onChange={(event) => {
                const value = event.target.value
                if (value === 'all' || value === 'true' || value === 'false') {
                  setStatus(value)
                }
              }}
            >
              <NativeSelectOption value='all'>{t('All')}</NativeSelectOption>
              <NativeSelectOption value='true'>
                {t('Enabled')}
              </NativeSelectOption>
              <NativeSelectOption value='false'>
                {t('Disabled')}
              </NativeSelectOption>
            </NativeSelect>
            <Button type='submit' variant='outline'>
              {t('Search')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              disabled={busy}
              onClick={reload}
            >
              {t('Refresh')}
            </Button>
          </form>
          {generated.length ? (
            <section
              className='bg-muted space-y-3 rounded-xl p-4'
              aria-label={t('Generated codes')}
            >
              <p className='font-medium'>{t('Generated codes')}</p>
              <pre className='max-h-64 overflow-auto text-sm'>
                {generated.map((code) => code.code).join('\n')}
              </pre>
              <Button
                variant='outline'
                disabled={busy}
                onClick={() =>
                  void copy(generated.map((code) => code.code).join('\n'))
                }
              >
                {t('Copy')}
              </Button>
            </section>
          ) : null}
          {loading ? (
            <p role='status'>{t('Loading…')}</p>
          ) : page?.items.length === 0 ? (
            <p>{t('No discount codes found.')}</p>
          ) : null}
          <div className='grid gap-4 lg:grid-cols-2'>
            {page?.items.map((code) => (
              <article
                key={code.id}
                className='bg-card space-y-4 rounded-xl border p-4'
              >
                <div className='flex items-start justify-between gap-3'>
                  <div className='min-w-0'>
                    <p className='font-mono break-all'>{code.code}</p>
                    <h2 className='text-muted-foreground text-sm break-all'>
                      {code.name}
                    </h2>
                  </div>
                  <span className='tabular-nums'>{code.discount_percent}%</span>
                </div>
                <dl className='grid grid-cols-2 gap-3 text-sm'>
                  <div>
                    <dt>{t('Minimum amount (Credits)')}</dt>
                    <dd className='break-all tabular-nums'>
                      {number(code.min_credit_units)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('Maximum uses (0 = unlimited)')}</dt>
                    <dd className='break-all tabular-nums'>
                      {number(code.max_uses)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('Used')}</dt>
                    <dd>{number(code.used_count)}</dd>
                  </div>
                  <div>
                    <dt>{t('Pending')}</dt>
                    <dd>{number(code.reserved_count)}</dd>
                  </div>
                </dl>
                <div className='flex items-center gap-3'>
                  <Switch
                    id={`discount-enabled-${code.id}`}
                    checked={code.enabled}
                    disabled={busy}
                    onCheckedChange={(enabled) =>
                      void run(async () => {
                        await setDiscountEnabled(scope, code, enabled)
                        if (current()) reload()
                      })
                    }
                  />
                  <Label htmlFor={`discount-enabled-${code.id}`}>
                    {code.enabled ? t('Enabled') : t('Disabled')}
                  </Label>
                </div>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    variant='outline'
                    size='sm'
                    disabled={busy}
                    onClick={() => open({ kind: 'edit', code })}
                  >
                    {t('Edit')}
                  </Button>
                  <Button
                    variant='outline'
                    size='sm'
                    disabled={busy}
                    onClick={() => open({ kind: 'delete', code })}
                  >
                    {t('Delete')}
                  </Button>
                  <Button
                    variant='ghost'
                    size='sm'
                    disabled={busy}
                    onClick={() => void copy(code.code)}
                  >
                    {t('Copy code')}
                  </Button>
                  <Button
                    variant='ghost'
                    size='sm'
                    disabled={busy}
                    onClick={() => void copy(buildDiscountCodeLink(code.code))}
                  >
                    {t('Copy link')}
                  </Button>
                </div>
              </article>
            ))}
          </div>
          <div className='flex gap-3'>
            <Button
              variant='outline'
              disabled={!previous.length || loading}
              onClick={() => {
                const before = previous.at(-1)
                if (before !== undefined) {
                  setPrevious(previous.slice(0, -1))
                  setAfter(before)
                }
              }}
            >
              {t('Previous')}
            </Button>
            <Button
              variant='outline'
              disabled={!page?.next_after || loading}
              onClick={() => {
                if (page?.next_after) {
                  setPrevious([...previous, after])
                  setAfter(page.next_after)
                }
              }}
            >
              {t('Next')}
            </Button>
          </div>
          <Dialog
            open={editor.kind !== 'closed'}
            onOpenChange={(open) => {
              if (!open && !busy) setEditor({ kind: 'closed' })
            }}
          >
            <DialogContent className='max-h-[90dvh] overflow-y-auto'>
              <DialogHeader>
                <DialogTitle>
                  {editor.kind === 'delete'
                    ? t('Delete')
                    : editor.kind === 'cleanup'
                      ? t('Delete exhausted codes')
                      : editor.kind === 'edit'
                        ? t('Edit')
                        : editor.kind === 'batch'
                          ? t('Generate codes')
                          : t('Create discount code')}
                </DialogTitle>
                <DialogDescription>
                  {editor.kind === 'delete'
                    ? t(
                        'Remove this code? Existing orders keep their original discount.'
                      )
                    : editor.kind === 'cleanup'
                      ? t(
                          'Remove exhausted codes? Codes reserved by unpaid orders will be kept.'
                        )
                      : t(
                          'Amounts are in Credits. Leave dates empty for immediate start and no expiry.'
                        )}
                </DialogDescription>
              </DialogHeader>
              {error ? (
                <p role='alert' className='text-destructive text-sm'>
                  {message}
                </p>
              ) : null}
              {editor.kind === 'delete' || editor.kind === 'cleanup' ? (
                <DialogFooter>
                  <Button
                    variant='outline'
                    disabled={busy}
                    onClick={() => setEditor({ kind: 'closed' })}
                  >
                    {t('Cancel')}
                  </Button>
                  <Button
                    variant='destructive'
                    disabled={busy}
                    onClick={() => {
                      const target = editor
                      void run(async () => {
                        if (target.kind === 'delete') {
                          await removeDiscount(scope, target.code)
                        } else await removeExhaustedDiscounts(scope)
                        if (current()) {
                          setEditor({ kind: 'closed' })
                          reload()
                        }
                      })
                    }}
                  >
                    {t('Delete')}
                  </Button>
                </DialogFooter>
              ) : (
                <form className='space-y-4' onSubmit={submit}>
                  {editor.kind !== 'batch'
                    ? field('code', t('Discount code'))
                    : field('count', t('Number of codes'), 'number')}
                  {field('name', t('Name'))}
                  <div className='grid gap-4 sm:grid-cols-2'>
                    {field('percent', t('Discount (%)'), 'number')}
                    {field('minimum', t('Minimum amount (Credits)'))}
                    {field('maximum', t('Maximum uses (0 = unlimited)'))}
                    {field('starts', t('Starts at'), 'datetime-local')}
                    {field('ends', t('Expires at'), 'datetime-local')}
                  </div>
                  <div className='flex items-center gap-3'>
                    <Switch
                      id='native-discount-enabled'
                      checked={form.enabled}
                      disabled={busy || Boolean(pending)}
                      onCheckedChange={(enabled) =>
                        setForm((old) => ({ ...old, enabled }))
                      }
                    />
                    <Label htmlFor='native-discount-enabled'>
                      {t('Enabled')}
                    </Label>
                  </div>
                  <DialogFooter>
                    <Button
                      type='button'
                      variant='outline'
                      disabled={busy}
                      onClick={() => setEditor({ kind: 'closed' })}
                    >
                      {t('Cancel')}
                    </Button>
                    <Button
                      type='submit'
                      disabled={busy || Boolean(pending) || storageFailed}
                    >
                      {editor.kind === 'batch'
                        ? t('Generate codes')
                        : t('Save')}
                    </Button>
                  </DialogFooter>
                </form>
              )}
            </DialogContent>
          </Dialog>
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
