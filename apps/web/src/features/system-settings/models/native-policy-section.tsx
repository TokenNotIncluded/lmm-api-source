/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { hashKey, useQuery } from '@tanstack/react-query'
import { useRef, useState, type ComponentType } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Dialog } from '@/components/dialog'
import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'
import {
  nativeRequest,
  NATIVE_ROOT,
  readNativeRoutingScope,
  stringifyNativeDocument,
  type NativeRoutingScope,
  type NativeVersion,
} from '@/features/channels/native-api'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

const path = `${NATIVE_ROOT}/configuration`

type Snapshot = {
  version: NativeVersion
  policy_sections?: string[]
  routing: { version: number; [key: string]: unknown }
  prices: { version: number; [key: string]: unknown }
  bindings: Record<string, unknown>
}

export type PolicyFormProps<T> = {
  policy: T
  preview: (next: T) => Promise<void>
  busy: boolean
}

type PolicySection<T> = {
  name: string
  draft: T
  project?: (value: T) => unknown
  onPublished?: () => void
  editor: ComponentType<PolicyFormProps<T>>
}

// Rust omits the empty channel_overrides map. Optional policies stay absent
// until an administrator publishes them.
function publishedPolicy(routing: Snapshot['routing'], name: string): unknown {
  return routing[name] ?? (name === 'channel_overrides' ? {} : null)
}

async function readSnapshot(scope: NativeRoutingScope) {
  const config = await nativeRequest<Snapshot>(scope, 'GET', path)
  if (
    !Number.isSafeInteger(config.version?.routing) ||
    config.version.routing !== config.routing?.version ||
    !Number.isSafeInteger(config.version.pricing) ||
    config.version.pricing !== config.prices?.version ||
    !config.bindings
  ) {
    throw new Error('The routing snapshot changed. Reload before continuing.')
  }
  return config
}

export function NativePolicySection<T>(props: PolicySection<T>) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeRoutingScope(auth)
  if (!scope) return <p role='alert'>{t('Administrator access required')}</p>
  return (
    <PolicyEditor
      key={JSON.stringify([
        scope.userId,
        scope.sessionId,
        scope.role,
        props.name,
      ])}
      scope={scope}
      {...props}
    />
  )
}

function PolicyEditor<T>({
  scope,
  name,
  draft,
  editor: Editor,
  project,
  onPublished,
}: PolicySection<T> & {
  scope: NativeRoutingScope
}) {
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: [
      'native-policy-editor',
      scope.userId,
      scope.sessionId,
      scope.role,
      name,
    ],
    queryFn: () => readSnapshot(scope),
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  const snapshot = query.data
  const [preview, setPreview] = useState<{
    input: Omit<Snapshot, 'version'> & { expected_version: NativeVersion }
    digest: string
    policy: T
  } | null>(null)
  const [error, setError] = useState('')
  const [reloadRequired, setReloadRequired] = useState(false)
  const [busy, setBusy] = useState(false)
  const pending = useRef(false)

  async function prepare(next: T) {
    if (!snapshot || pending.current || reloadRequired) return
    if (
      hashKey([next]) === hashKey([publishedPolicy(snapshot.routing, name)])
    ) {
      toast.info(t('No changes to save'))
      return
    }
    pending.current = true
    setBusy(true)
    setError('')
    try {
      const input = {
        expected_version: snapshot.version,
        routing: {
          ...snapshot.routing,
          version: snapshot.version.routing + 1,
          [name]: next,
        },
        prices: snapshot.prices,
        bindings: snapshot.bindings,
      }
      const receipt = await nativeRequest<{
        saved: boolean
        expected_version: NativeVersion
        routing: Snapshot['routing']
        confirmation_digest: string
      }>(scope, 'POST', `${path}/preview`, input)
      if (
        receipt.saved !== false ||
        receipt.expected_version?.routing !== snapshot.version.routing ||
        receipt.expected_version?.pricing !== snapshot.version.pricing ||
        receipt.routing?.version !== input.routing.version ||
        (!receipt.routing[name] &&
          !(
            name === 'channel_overrides' && hashKey([next]) === hashKey([{}])
          )) ||
        !/^[0-9a-f]{64}$/.test(receipt.confirmation_digest)
      ) {
        throw new Error('The preview did not return a confirmation digest.')
      }
      setPreview({
        input,
        digest: receipt.confirmation_digest,
        policy: publishedPolicy(receipt.routing, name) as T,
      })
    } catch (cause) {
      setError(getServerErrorMessage(cause) ?? t('Failed to load data'))
    } finally {
      pending.current = false
      setBusy(false)
    }
  }

  async function publish() {
    if (!preview || pending.current || reloadRequired) return
    pending.current = true
    setBusy(true)
    setError('')
    setPreview(null)
    try {
      await nativeRequest(scope, 'PUT', path, {
        ...preview.input,
        confirmation_digest: preview.digest,
      })
      const result = await query.refetch()
      if (
        result.error ||
        !result.data ||
        result.data.version.routing < preview.input.routing.version ||
        (result.data.version.routing === preview.input.routing.version &&
          hashKey([publishedPolicy(result.data.routing, name)]) !==
            hashKey([preview.policy]))
      ) {
        throw new Error('The save result is unknown. Reload before continuing.')
      }
      toast.success(t('Saved successfully'))
      onPublished?.()
    } catch (cause) {
      setReloadRequired(true)
      setError(
        getServerErrorMessage(cause) ??
          t('The save result is unknown. Reload before continuing.')
      )
    } finally {
      pending.current = false
      setBusy(false)
    }
  }
  if (query.isPending) return <LoadingState />
  if (!snapshot) {
    return (
      <ErrorState
        description={
          getServerErrorMessage(query.error) ?? t('Failed to load data')
        }
        onRetry={() => void query.refetch()}
      />
    )
  }
  if (!snapshot.policy_sections?.includes(name)) {
    return <p role='status'>{t('Capability unavailable')}</p>
  }

  return (
    <div className='space-y-4'>
      <p className='text-muted-foreground text-sm'>
        {t('Current version')} {snapshot.version.routing}
        {snapshot.routing[name] == null && <> · {t('Draft')}</>}
      </p>
      {error && (
        <div role='alert' className='space-y-2 text-sm'>
          <p>{error}</p>
          <Button
            variant='outline'
            disabled={busy || query.isFetching}
            onClick={async () => {
              const result = await query.refetch()
              if (!result.error) {
                setError('')
                setReloadRequired(false)
              }
            }}
          >
            {t('Reload configuration')}
          </Button>
        </div>
      )}
      <Editor
        key={snapshot.version.routing}
        policy={(snapshot.routing[name] as T | null | undefined) ?? draft}
        preview={prepare}
        busy={busy || reloadRequired}
      />
      <Dialog
        open={preview !== null}
        onOpenChange={(open) => {
          if (!open) setPreview(null)
        }}
        title={t('Configuration preview')}
        footer={
          <>
            <Button variant='outline' onClick={() => setPreview(null)}>
              {t('Cancel')}
            </Button>
            <Button disabled={busy} onClick={() => void publish()}>
              {t('Confirm')}
            </Button>
          </>
        }
      >
        {preview && (
          <div className='grid min-w-0 gap-4 sm:grid-cols-2'>
            <div>
              <h3 className='text-sm font-medium'>
                {t('Current version')} {snapshot.version.routing}
              </h3>
              <pre className='max-h-72 overflow-auto text-xs break-all whitespace-pre-wrap'>
                {stringifyNativeDocument(
                  project
                    ? project(
                        (snapshot.routing[name] as T | null | undefined) ??
                          draft
                      )
                    : (snapshot.routing[name] ?? null),
                  2
                )}
              </pre>
            </div>
            <div>
              <h3 className='text-sm font-medium'>
                {t('Draft')} {preview.input.routing.version}
              </h3>
              <pre className='max-h-72 overflow-auto text-xs break-all whitespace-pre-wrap'>
                {stringifyNativeDocument(
                  project ? project(preview.policy) : preview.policy,
                  2
                )}
              </pre>
            </div>
          </div>
        )}
      </Dialog>
    </div>
  )
}
