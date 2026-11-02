/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { zodResolver } from '@hookform/resolvers/zod'
import { useEffect, useRef, useState } from 'react'
import { useForm, type Resolver } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Form } from '@/components/ui/form'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useAuthStore } from '@/stores/auth-store'

import { FormNavigationGuard } from '../components/form-navigation-guard'
import { SettingsForm } from '../components/settings-form-layout'
import { TrustLevelBenefitsFields } from './trust-level-benefits-fields'
import {
  trustLevelBenefitsSchema,
  type TrustLevelBenefitsConfig,
} from './trust-level-benefits-schema'
import {
  TrustPolicyEditorClient,
  trustPolicyDraft,
  trustPolicyScope,
  type TrustPolicyRead,
  type TrustPolicyPreview,
  type TrustPolicyResolution,
} from './trust-policy-editor-client'
import { TrustPolicyReview } from './trust-policy-editor-review'

export function TrustPolicyEditor({ enabled }: { enabled: boolean }) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  if (!enabled || auth.user?.role !== 100 || !auth.session?.sid) {
    return (
      <Alert>
        <AlertDescription>
          {t('Trust policy editing is unavailable for this session.')}
        </AlertDescription>
      </Alert>
    )
  }
  // Keep the original token on same-actor session renewal so resolve remains possible.
  return <NativeEditor key={auth.user.id} />
}

function NativeEditor() {
  const { t } = useTranslation()
  const [generation, setGeneration] = useState(0)
  const [loaded, setLoaded] = useState<{
    client: TrustPolicyEditorClient
    read: TrustPolicyRead
  } | null>(null)
  const [error, setError] = useState(false)
  const [confirmedRevision, setConfirmedRevision] = useState<number | null>(
    null
  )
  useEffect(() => {
    const scope = trustPolicyScope()
    if (!scope) {
      setError(true)
      return
    }
    const client = new TrustPolicyEditorClient(scope)
    let live = true
    setLoaded(null)
    setError(false)
    void client
      .read()
      .then((read) => {
        if (live) setLoaded({ client, read })
      })
      .catch(() => {
        if (live) setError(true)
      })
    return () => {
      live = false
      client.dispose()
    }
  }, [generation])
  const reload = () => {
    setLoaded(null)
    setGeneration((n) => n + 1)
  }
  return (
    <div className='space-y-4'>
      {confirmedRevision !== null && (
        <Alert>
          <AlertDescription role='status'>
            {t('Trust policy confirmed at revision {{revision}}.', {
              revision: confirmedRevision,
            })}
          </AlertDescription>
        </Alert>
      )}
      {!loaded && (
        <p role='status'>
          {t(
            error
              ? 'Unable to load the current trust policy.'
              : 'Loading trust policy…'
          )}
        </p>
      )}
      {error && (
        <Button variant='outline' onClick={reload}>
          {t('Retry')}
        </Button>
      )}
      {loaded && (
        <LoadedEditor
          client={loaded.client}
          read={loaded.read}
          onReload={reload}
          onConfirmed={(revision) => {
            setConfirmedRevision(revision)
            reload()
          }}
        />
      )}
    </div>
  )
}

function LoadedEditor({
  client,
  read,
  onConfirmed,
  onReload,
}: {
  client: TrustPolicyEditorClient
  read: TrustPolicyRead
  onReload: () => void
  onConfirmed: (revision: number) => void
}) {
  const { t } = useTranslation()
  const editors = read.editors.filter((editor) => editor.status === 'active')
  let draft: TrustLevelBenefitsConfig | null = null
  try {
    draft = trustPolicyDraft(read)
  } catch {
    /* Fail closed instead of rounding a financial ratio. */
  }
  const webAuthn =
    typeof window !== 'undefined' &&
    window.isSecureContext === true &&
    typeof window.PublicKeyCredential === 'function' &&
    typeof navigator.credentials?.get === 'function'
  if (!editors.length || !draft || !webAuthn) {
    return (
      <div className='space-y-4'>
        <Alert>
          <AlertDescription>
            {t(
              !editors.length
                ? 'No active owner-registered editor credential. This policy is read only.'
                : !webAuthn
                  ? 'A secure browser with WebAuthn is required to edit this policy.'
                  : 'The policy cannot be represented exactly in this editor.'
            )}
          </AlertDescription>
        </Alert>
        <TrustPolicyReview
          current={read.policy}
          currentConsole={read.console}
        />
        <Button variant='outline' onClick={onReload}>
          {t('Reload current policy')}
        </Button>
      </div>
    )
  }
  return (
    <EditorForm
      client={client}
      read={read}
      defaultValues={draft}
      editors={editors}
      onConfirmed={onConfirmed}
      onReload={onReload}
    />
  )
}

function EditorForm({
  client,
  read,
  defaultValues,
  editors,
  onConfirmed,
  onReload,
}: {
  client: TrustPolicyEditorClient
  read: TrustPolicyRead
  defaultValues: TrustLevelBenefitsConfig
  editors: TrustPolicyRead['editors']
  onReload: () => void
  onConfirmed: (revision: number) => void
}) {
  const { t } = useTranslation()
  const form = useForm<TrustLevelBenefitsConfig>({
    defaultValues,
    resolver: zodResolver(
      trustLevelBenefitsSchema
    ) as Resolver<TrustLevelBenefitsConfig>,
  })
  const [review, setReview] = useState<TrustPolicyPreview | null>(null)
  const [editor, setEditor] = useState(editors[0].editor_id)
  const [accepted, setAccepted] = useState(false)
  const [attempted, setAttempted] = useState(false)
  const [busy, setBusy] = useState(false)
  const lock = useRef(false)
  const [failed, setFailed] = useState(false)
  const [resolution, setResolution] = useState<TrustPolicyResolution | null>(
    null
  )
  const run = async (operation: () => Promise<void>) => {
    if (lock.current) return
    lock.current = true
    setBusy(true)
    setFailed(false)
    try {
      await operation()
    } catch {
      setFailed(true)
    } finally {
      lock.current = false
      setBusy(false)
    }
  }
  const finish = (result: TrustPolicyResolution) => {
    setResolution(result)
    if (result.state === 'confirmed' && result.revision !== null) {
      onConfirmed(result.revision)
    }
  }
  const preview = form.handleSubmit((data) =>
    run(async () => {
      setReview(await client.preview(data))
      setAccepted(false)
      setResolution(null)
      setAttempted(false)
    })
  )
  const restart = () => {
    client.discard()
    setReview(null)
    setAccepted(false)
    setResolution(null)
    setAttempted(false)
    setFailed(false)
  }
  return (
    <div className='space-y-4'>
      <FormNavigationGuard when={form.formState.isDirty || review !== null} />
      <p className='text-muted-foreground text-sm'>
        {t('Policy revision {{revision}}', {
          revision: review?.preview.base_revision ?? read.revision,
        })}
      </p>
      {(review
        ? review.preview.current_console === null
        : read.console === null) && (
        <Alert>
          <AlertDescription>
            {t(
              'Historical benefit descriptions are unknown. Your selections create new descriptions; they do not reconstruct past values.'
            )}
          </AlertDescription>
        </Alert>
      )}
      {failed && (
        <Alert variant='destructive'>
          <AlertDescription role='alert'>
            {t(
              review
                ? 'The request failed. Check the status of this review before retrying.'
                : 'Unable to preview the policy. Reload if the editing session changed.'
            )}
          </AlertDescription>
        </Alert>
      )}
      {!review ? (
        <Form {...form}>
          <SettingsForm
            onSubmit={(event) => {
              event.preventDefault()
              void preview()
            }}
          >
            <TrustLevelBenefitsFields
              exactRatios
              form={form}
              defaultValues={defaultValues}
              saving={busy}
            />
            <div className='flex flex-wrap gap-2'>
              <Button
                type='button'
                variant='outline'
                disabled={busy}
                onClick={() => form.reset(defaultValues)}
              >
                {t('Reset')}
              </Button>
              <Button
                type='button'
                variant='outline'
                disabled={busy}
                onClick={onReload}
              >
                {t('Reload current policy')}
              </Button>
              <Button type='submit' disabled={busy}>
                {t('Preview trust policy changes')}
              </Button>
            </div>
          </SettingsForm>
        </Form>
      ) : (
        <section
          className='space-y-4'
          aria-label={t('Review trust policy changes')}
        >
          <TrustPolicyReview
            current={review.preview.current}
            currentConsole={review.preview.current_console}
            proposed={review.preview.proposed}
            proposedConsole={review.preview.proposed_console}
          />
          {!attempted && (
            <>
              <label
                htmlFor='trust-policy-editor-id'
                className='text-sm font-medium'
              >
                {t('Owner-registered editor credential')}
              </label>
              <NativeSelect
                id='trust-policy-editor-id'
                value={editor}
                disabled={busy}
                onChange={(event) => setEditor(event.target.value)}
              >
                {editors.map((item) => (
                  <NativeSelectOption
                    key={item.editor_id}
                    value={item.editor_id}
                  >
                    {item.editor_id}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
              <div className='flex items-center gap-2'>
                <Checkbox
                  id='trust-policy-reviewed'
                  checked={accepted}
                  disabled={busy}
                  onCheckedChange={(value) => setAccepted(value === true)}
                />
                <label htmlFor='trust-policy-reviewed' className='text-sm'>
                  {t('I reviewed the current and proposed values.')}
                </label>
              </div>
            </>
          )}
          {resolution && resolution.state !== 'confirmed' && (
            <Alert>
              <AlertDescription role='status'>
                {t(
                  resolution.state === 'pending'
                    ? 'The change is pending, not confirmed. Check its status again.'
                    : 'The review is unknown or expired. Create a new preview and review it again.'
                )}
              </AlertDescription>
            </Alert>
          )}
          <div className='flex flex-wrap gap-2'>
            <Button
              type='button'
              variant='outline'
              disabled={busy || !client.canDiscard}
              onClick={restart}
            >
              {t('Start a new review')}
            </Button>
            {!attempted ? (
              <Button
                type='button'
                disabled={busy || !accepted}
                onClick={() => {
                  if (!accepted) return
                  void run(async () => {
                    setAttempted(true)
                    finish(await client.confirm(editor))
                  })
                }}
              >
                {t('Confirm changes with security key')}
              </Button>
            ) : (
              <Button
                type='button'
                disabled={busy}
                onClick={() =>
                  void run(async () => finish(await client.resolve()))
                }
              >
                {t('Check review status')}
              </Button>
            )}
          </div>
        </section>
      )}
    </div>
  )
}
