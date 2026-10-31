import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { isAxiosError, isCancel } from 'axios'
/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Markdown } from '@/components/ui/markdown'
import { Textarea } from '@/components/ui/textarea'
import { formatTimestampToDate } from '@/lib/format'
import { useAuthStore } from '@/stores/auth-store'

import {
  canManageReleaseNotes,
  isCurrentReleaseNoteScope,
  listReleaseNotes,
  publishReleaseNote,
  RELEASE_NOTE_MAX_BYTES,
  releaseNoteInputSchema,
  type ReleaseNoteScope,
} from './api'

function ReleaseNotesWorkspace({ scope }: { scope: ReleaseNoteScope }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const formId = useId()
  const submitting = useRef(false)
  const [draft, setDraft] = useState({ version: '', content: '' })
  const [submitted, setSubmitted] = useState(false)
  const queryKey = [
    'release-notes',
    'admin',
    scope.userId,
    scope.sessionId,
    scope.role,
  ] as const

  const history = useQuery({
    queryKey,
    queryFn: ({ signal }) => listReleaseNotes(scope, signal),
    retry: false,
    refetchOnWindowFocus: false,
  })

  const publish = useMutation({
    mutationFn: (input: { version: string; content: string }) =>
      publishReleaseNote(input, scope),
    retry: false,
    onSuccess: async () => {
      if (!isCurrentReleaseNoteScope(scope)) return

      // An initial history request may have started before publication.
      await queryClient.cancelQueries({ queryKey, exact: true })
      if (!isCurrentReleaseNoteScope(scope)) return
      void queryClient.invalidateQueries({ queryKey, exact: true })
    },
    onSettled: () => {
      submitting.current = false
    },
  })

  const validation = releaseNoteInputSchema.safeParse(draft)
  const issues = submitted && !validation.success ? validation.error.issues : []
  const versionError = issues.some((issue) => issue.path[0] === 'version')
    ? draft.version.trim()
      ? t('Invalid input')
      : t('Version is required')
    : undefined
  const contentError = issues.some((issue) => issue.path[0] === 'content')
    ? draft.content.trim()
      ? t('Invalid input')
      : t('Changelog is required')
    : undefined
  const contentBytes = new TextEncoder().encode(draft.content).byteLength

  function failureMessage(error: unknown, fallback: string) {
    const status = isAxiosError(error) ? error.response?.status : undefined
    if (status === 401) return t('Your session has expired. Sign in again.')
    if (status === 403) return t('A current administrator session is required.')
    if (status === 503) return t('The extension service is unavailable (503).')
    return fallback
  }

  function updateDraft(field: 'version' | 'content', value: string) {
    if (submitting.current || !isCurrentReleaseNoteScope(scope)) return
    setDraft((current) => ({ ...current, [field]: value }))
    publish.reset()
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (submitting.current || !isCurrentReleaseNoteScope(scope)) return
    setSubmitted(true)
    if (!validation.success) return

    submitting.current = true
    publish.mutate(validation.data)
  }

  function refreshHistory() {
    if (isCurrentReleaseNoteScope(scope)) void history.refetch()
  }

  return (
    <div className='mx-auto w-full max-w-5xl space-y-8'>
      <section
        aria-labelledby={`${formId}-publish-title`}
        className='space-y-5'
      >
        <div className='space-y-2'>
          <h2 id={`${formId}-publish-title`} className='text-lg font-semibold'>
            {t('Publish version update')}
          </h2>
          <p className='text-muted-foreground text-sm'>
            {t(
              'Every released version must include a version number and changelog. Users will see it after their next login.'
            )}
          </p>
        </div>

        <form onSubmit={submit} noValidate className='space-y-5'>
          <div className='max-w-sm space-y-2'>
            <Label htmlFor={`${formId}-version`}>{t('Version')}</Label>
            <Input
              id={`${formId}-version`}
              value={draft.version}
              onChange={(event) => updateDraft('version', event.target.value)}
              autoComplete='off'
              spellCheck={false}
              required
              disabled={publish.isPending}
              aria-invalid={Boolean(versionError)}
              aria-describedby={
                versionError ? `${formId}-version-error` : undefined
              }
            />
            {versionError && (
              <p
                id={`${formId}-version-error`}
                role='alert'
                className='text-destructive text-sm'
              >
                {versionError}
              </p>
            )}
          </div>

          <div className='grid min-w-0 gap-6 lg:grid-cols-2'>
            <div className='min-w-0 space-y-2'>
              <Label htmlFor={`${formId}-content`}>{t('Changelog')}</Label>
              <Textarea
                id={`${formId}-content`}
                value={draft.content}
                onChange={(event) => updateDraft('content', event.target.value)}
                className='min-h-64 resize-y font-mono text-sm'
                rows={12}
                required
                disabled={publish.isPending}
                aria-invalid={Boolean(contentError)}
                aria-describedby={[
                  `${formId}-content-bytes`,
                  contentError ? `${formId}-content-error` : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
              />
              <p
                id={`${formId}-content-bytes`}
                className={
                  contentBytes > RELEASE_NOTE_MAX_BYTES
                    ? 'text-destructive text-xs tabular-nums'
                    : 'text-muted-foreground text-xs tabular-nums'
                }
              >
                {contentBytes.toLocaleString()} /{' '}
                {RELEASE_NOTE_MAX_BYTES.toLocaleString()} B (UTF-8)
              </p>
              {contentError && (
                <p
                  id={`${formId}-content-error`}
                  role='alert'
                  className='text-destructive text-sm'
                >
                  {contentError}
                </p>
              )}
            </div>

            <section
              aria-labelledby={`${formId}-preview-title`}
              className='min-w-0 space-y-3 rounded-xl border p-4'
            >
              <h3
                id={`${formId}-preview-title`}
                className='text-sm font-medium'
              >
                {t('Preview')}
              </h3>
              {draft.content.trim() ? (
                <Markdown>{draft.content}</Markdown>
              ) : (
                <p className='text-muted-foreground text-sm'>
                  {t('Enter a changelog to preview it here.')}
                </p>
              )}
            </section>
          </div>

          {publish.isError && !isCancel(publish.error) && (
            <Alert variant='destructive'>
              <AlertDescription>
                {failureMessage(publish.error, t('Request failed'))}
              </AlertDescription>
            </Alert>
          )}

          {publish.isSuccess && (
            <Alert role='status'>
              <AlertDescription>
                {t('Version {{version}} revision {{revision}} published', {
                  version: publish.data.version,
                  revision: publish.data.revision,
                })}
                <span className='mt-1 block text-xs'>
                  {t('Published {{date}}', {
                    date: formatTimestampToDate(publish.data.published_at),
                  })}
                </span>
              </AlertDescription>
            </Alert>
          )}

          <Button
            type='submit'
            disabled={publish.isPending}
            className='min-h-11'
          >
            {publish.isPending ? t('Publishing...') : t('Publish update')}
          </Button>
        </form>
      </section>

      <section
        aria-labelledby={`${formId}-history-title`}
        className='space-y-4 border-t pt-6'
      >
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <h2 id={`${formId}-history-title`} className='text-lg font-semibold'>
            {t('Release history')}
          </h2>
          <Button
            type='button'
            variant='outline'
            onClick={refreshHistory}
            disabled={history.isFetching}
            className='min-h-11'
          >
            {history.isFetching
              ? t('Refreshing...')
              : history.isError
                ? t('Retry')
                : t('Refresh')}
          </Button>
        </div>

        {history.isPending && (
          <p role='status' className='text-muted-foreground text-sm'>
            {t('Loading release history...')}
          </p>
        )}
        {history.isError && (
          <Alert variant='destructive'>
            <AlertDescription>
              {failureMessage(
                history.error,
                t('Failed to load release history')
              )}
            </AlertDescription>
          </Alert>
        )}
        {history.isSuccess &&
          (history.data.length === 0 ? (
            <p className='text-muted-foreground text-sm'>
              {t('No version updates have been published yet.')}
            </p>
          ) : (
            <div className='divide-y border-y'>
              {history.data.map((note) => (
                <details key={note.id} className='min-w-0'>
                  <summary className='cursor-pointer py-4 focus-visible:outline-2 focus-visible:outline-offset-2'>
                    <span className='ml-2 inline-flex max-w-full flex-wrap items-center gap-x-3 gap-y-2 align-middle'>
                      <span className='font-medium [overflow-wrap:anywhere]'>
                        {note.version}
                      </span>
                      <Badge variant='secondary'>
                        {t('Revision {{revision}}', {
                          revision: note.revision,
                        })}
                      </Badge>
                      <time
                        dateTime={new Date(
                          note.published_at * 1000
                        ).toISOString()}
                        className='text-muted-foreground text-xs'
                      >
                        {t('Published {{date}}', {
                          date: formatTimestampToDate(note.published_at),
                        })}
                      </time>
                    </span>
                  </summary>
                  <Markdown className='pb-5'>{note.content}</Markdown>
                </details>
              ))}
            </div>
          ))}
      </section>
    </div>
  )
}

export function ReleaseNotesAdmin() {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope: ReleaseNoteScope | null =
    auth.user &&
    canManageReleaseNotes(auth.user) &&
    Number.isSafeInteger(auth.user.id) &&
    auth.user.id > 0 &&
    auth.session?.sid &&
    auth.accessToken
      ? {
          userId: auth.user.id,
          sessionId: auth.session.sid,
          role: auth.user.role,
        }
      : null

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Version updates')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {scope ? (
          <ReleaseNotesWorkspace
            key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
            scope={scope}
          />
        ) : (
          <Alert>
            <AlertDescription>
              {t('A current administrator session is required.')}
            </AlertDescription>
          </Alert>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
