/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  ChevronDown,
  KeyRound,
  LayoutGrid,
  List,
  Pencil,
  Play,
  Plus,
  RefreshCw,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Dialog } from '@/components/dialog'
import { EmptyState } from '@/components/empty-state'
import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Textarea } from '@/components/ui/textarea'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { toIntlLocale } from '@/i18n/languages'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  setNativeChannelEnabled,
  setNativeChannelsEnabled,
  setNativeChannelsTag,
} from '../api'
import { nativeModelLogo, resolveNativeProvider } from '../lib/native-provider'
import { nativeProtocolLabel } from '../lib/native-routing-form'
import {
  assertNativeDraftTimeouts,
  nativeTimeoutErrorMessage,
} from '../lib/native-target-timeout'
import {
  copyNativeChannel,
  getNativeConfiguration,
  type NativeConfiguration,
  nativeRequest as scopedNativeRequest,
  readNativeRoutingScope,
  type NativeRoutingScope,
  NATIVE_ROOT,
  parseNativeDocument,
  stringifyNativeDocument,
  updateNativeChannels,
  type NativeChannelBatchPatch,
  type NativeModelRoute,
  type NativePriceDraft,
  type NativeUpstream,
  type NativeVendor,
  type NativeVersion,
} from '../native-api'
import {
  isChannelSnapshotChanged,
  getNativeChannelPage,
  type NativeChannelSortInput,
  type NativeChannelFilters,
} from '../native-channel-statistics'
import { useChannelOps } from '../use-channel-ops'
import { NativeBalanceBatchDialog } from './dialogs/native-balance-batch-dialog'
import { NativeBalanceConfigDialog } from './dialogs/native-balance-config-dialog'
import { NativeBalanceDialog } from './dialogs/native-balance-dialog'
import { NativeChannelOverridesDialog } from './dialogs/native-channel-overrides-dialog'
import { NativeChannelUsageDialog } from './dialogs/native-channel-usage-dialog'
import { NativeKeyManageDialog } from './dialogs/native-key-manage-dialog'
import { NativeChannelBatchEditor } from './native-channel-batch-editor'
import {
  NativeChannelFields,
  NativeChannelTagField,
} from './native-channel-fields'
import { NativeChannelFilterFields } from './native-channel-filter-fields'
import { NativeChannelOnboarding } from './native-channel-onboarding'
import {
  NativeChannelOperationsSummary,
  NativeChannelSortFields,
  NativeChannelStatisticsRow,
  NativeChannelStatisticsSummary,
} from './native-channel-statistics'
import {
  defaultModelDisplayName,
  newModelTarget,
  readNativeModelDraft,
  replaceModelTargets,
} from './native-model-draft'
import { NativeModelFields } from './native-model-fields'
import { NativePriceRuleList } from './native-price-rule-list'
import { NativeProviderLogo } from './native-provider-logo'

import './native-channel-list.css'

type Section = 'channels' | 'models' | 'vendors' | 'prices' | 'unpriced'
type Editor = {
  title: string
  version: NativeVersion
  path: string
  method: 'POST' | 'PUT'
  document: string
  previewPath?: string
  modelEditing?: boolean
  secret?: boolean
  readOnly?: boolean
  channelId?: number
  fetchCatalogOnOpen?: boolean
  trigger?: HTMLButtonElement
}
type EditorDestination =
  | { editor: Editor }
  | { section: Section }
  | { close: true }
const sections: Array<[Section, string]> = [
  ['channels', 'Channels'],
  ['models', 'Models'],
  ['vendors', 'Vendor'],
  ['prices', 'Pricing'],
  ['unpriced', 'Unpriced models'],
]
const pretty = (value: unknown) => stringifyNativeDocument(value, 2)
const nextId = (ids: number[]) =>
  ids.reduce((maximum, id) => Math.max(maximum, id), 0) + 1

/** Administrative edits always start from a server version and require CAS. */
export function NativeRoutingManagement({
  initialSection = 'channels',
  configurationWritable = true,
}: {
  initialSection?: Section
  configurationWritable?: boolean
}) {
  const { t } = useTranslation()
  const runtime = useRuntimeCapabilities()
  const localCapabilities = useChannelOps(!runtime)
  const capabilities = runtime?.channels ?? localCapabilities
  const capabilityPhase = useCapabilityFreshness(capabilities)
  const secretsAllowed =
    capabilityPhase === 'fresh' &&
    capabilities.data?.data?.native_secrets === true
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeRoutingScope(auth)
  if (!scope) {
    return (
      <Alert variant='destructive' role='alert'>
        <AlertDescription>
          {t('Administrator access required')}
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <NativeRoutingWorkspace
      key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
      scope={scope}
      configurationWritable={
        configurationWritable &&
        capabilityPhase === 'fresh' &&
        capabilities.data?.data?.native_configuration === true
      }
      secretsAllowed={secretsAllowed}
      bulkKeyImportAllowed={
        secretsAllowed && capabilities.data?.data?.bulk_key_import === true
      }
      accounting={{
        observations:
          capabilityPhase === 'fresh' &&
          capabilities.data?.data?.native_channel_observations === true,
        usage:
          capabilityPhase === 'fresh' &&
          capabilities.data?.data?.native_channel_usage === true,
        counters:
          capabilityPhase === 'fresh' &&
          capabilities.data?.data?.native_channel_counters === true,
        balanceProbe:
          capabilityPhase === 'fresh' &&
          capabilities.data?.data?.native_balance_probe === true,
        preserveCounters:
          capabilityPhase === 'fresh' &&
          capabilities.data?.data?.copy_preserve_counters === true,
      }}
      initialSection={initialSection}
    />
  )
}
function NativeRoutingWorkspace({
  initialSection,
  scope,
  configurationWritable,
  secretsAllowed,
  bulkKeyImportAllowed,
  accounting,
}: {
  initialSection: Section
  scope: NativeRoutingScope
  configurationWritable: boolean
  secretsAllowed: boolean
  bulkKeyImportAllowed: boolean
  accounting: {
    observations: boolean
    usage: boolean
    counters: boolean
    balanceProbe: boolean
    preserveCounters: boolean
  }
}) {
  const nativeRequest = <T,>(
    method: 'GET' | 'POST' | 'PUT' | 'DELETE',
    path: string,
    data?: unknown
  ) => {
    if (method !== 'GET' && !configurationWritable) {
      return Promise.reject(
        new Error(
          t(
            'Capability information is not current. Refresh before making changes.'
          )
        )
      )
    }
    return scopedNativeRequest<T>(scope, method, path, data)
  }
  const { t, i18n } = useTranslation()
  const client = useQueryClient()
  const [copyReset, setCopyReset] = useState(true)
  const [overrideId, setOverrideId] = useState<number | null>(null)
  const [balanceChannel, setBalanceChannel] = useState<NativeUpstream | null>(
    null
  )
  const [balanceBatchChannels, setBalanceBatchChannels] = useState<Array<{
    id: number
    name: string
  }> | null>(null)
  const [balanceConfigChannel, setBalanceConfigChannel] =
    useState<NativeUpstream | null>(null)
  const [copySuffix, setCopySuffix] = useState('_copy')
  const copyPending = useRef(false)
  const [copyChannel, setCopyChannel] = useState<NativeUpstream | null>(null)
  const [onboardingId, setOnboardingId] = useState<number | null>(null)
  const [onboardingModel, setOnboardingModel] = useState<string>()
  const [usageChannel, setUsageChannel] = useState<NativeUpstream | null>(null)
  const [copyError, setCopyError] = useState<string | null>(null)
  const [section, setSection] = useState<Section>(initialSection)
  const [page, setPage] = useState(1)
  const [search, setSearch] = useState('')
  const [channelLayout, setChannelLayout] = useState<'cards' | 'list'>('list')
  const [channelFilters, setChannelFilters] = useState<NativeChannelFilters>({
    status: 'all',
    group: '',
    protocol: '',
  })
  const [channelSelection, setChannelSelection] = useState<{
    snapshot: string
    ids: number[]
  }>({ snapshot: '', ids: [] })
  const [batchTagEditor, setBatchTagEditor] = useState<{
    snapshot: string
    value: string
  } | null>(null)
  const [batchRoutingEditor, setBatchRoutingEditor] = useState<string | null>(
    null
  )
  const [channelSort, setChannelSort] = useState<
    Pick<NativeChannelSortInput, 'sort' | 'order' | 'unit'>
  >({ sort: 'id', order: 'asc', unit: '' })
  const [listingRevision, setListingRevision] = useState(0)
  const snapshotAnchor = useRef<{ selection: string; digest: string } | null>(
    null
  )
  // The ref supplies continuation requests; state owns what may be rendered.
  const [channelListing, setChannelListing] = useState<{
    anchor: { selection: string; digest: string } | null
    balanceUnits: string[]
  }>({ anchor: null, balanceUnits: [] })
  const [modelAfterSave, setModelAfterSave] = useState<{
    model: string
    group: string
  } | null>(null)
  const [editor, setEditor] = useState<Editor | null>(null)
  const editorBaseline = useRef('')
  const [editorDestination, setEditorDestination] =
    useState<EditorDestination | null>(null)
  const editorRef = useRef<HTMLFormElement>(null)
  const channelEditorTrigger = useRef<HTMLButtonElement | null>(null)
  const discardFocus = useRef<HTMLButtonElement | false | null>(null)
  useEffect(() => {
    if (!editor?.channelId) {
      editorRef.current?.scrollIntoView?.({ block: 'start' })
    }
    editorRef.current
      ?.querySelector<HTMLInputElement>('input')
      ?.focus({ preventScroll: true })
  }, [editor?.path, editor?.channelId])
  const [secret, setSecret] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const writeDisabled = busy || !configurationWritable
  const operationPending = useRef(false)
  const [preview, setPreview] = useState<{
    confirmation_digest: string
    [key: string]: unknown
  } | null>(null)
  const [confirmation, setConfirmation] = useState<string | null>(null)
  const [routingChannelId, setRoutingChannelId] = useState<number | null>(null)
  const [groupEditor, setGroupEditor] = useState<{
    id: number
    version: NativeVersion
    groups: string[]
  } | null>(null)
  const [channelToDelete, setChannelToDelete] = useState<{
    id: number
    name: string
    version: NativeVersion
    routes: string[]
  } | null>(null)
  const [result, setResult] = useState<unknown>(null)
  const [keyChannel, setKeyChannel] = useState<number | null>(null)
  const queryKey = [
    'native-routing-configuration',
    scope.userId,
    scope.sessionId,
    scope.role,
  ] as const
  const query = useQuery({
    queryKey,
    queryFn: () => getNativeConfiguration(scope),
    retry: false,
  })
  useEffect(
    () => () => {
      client.removeQueries({
        queryKey: [
          'native-channel-statistics',
          scope.userId,
          scope.sessionId,
          scope.role,
        ],
      })
      client.removeQueries({
        queryKey: [
          'native-routing-configuration',
          scope.userId,
          scope.sessionId,
          scope.role,
        ],
        exact: true,
      })
    },
    [client, scope.userId, scope.sessionId, scope.role]
  )
  const configuration = query.data
  const editorStale = !!(
    editor &&
    (editor.channelId || editor.modelEditing !== undefined) &&
    configuration &&
    (editor.version.routing !== configuration.version.routing ||
      editor.version.pricing !== configuration.version.pricing)
  )
  const channelSelectionSnapshot = JSON.stringify([
    channelSort,
    channelFilters,
    configuration?.version,
    page,
    scope.sessionId,
    scope.userId,
    search,
    section,
  ])
  const selectedChannelIds =
    channelSelection.snapshot === channelSelectionSnapshot
      ? channelSelection.ids
      : []
  function updateChannelSelection(ids: number[]) {
    setBatchRoutingEditor(null)
    setChannelSelection({ snapshot: channelSelectionSnapshot, ids })
    if (ids.length === 0) setBatchTagEditor(null)
  }
  const selection = JSON.stringify([
    scope,
    query.data?.version,
    channelSort,
    channelFilters,
    search,
    listingRevision,
  ])
  const sortedChannels = useQuery({
    // Every page visit must validate its digest, even with the app's 10s cache.
    staleTime: 0,
    refetchOnMount: 'always',
    queryKey: [
      'native-channel-statistics',
      scope.userId,
      scope.sessionId,
      scope.role,
      query.data?.version,
      channelSort,
      channelFilters,
      page,
      search,
      listingRevision,
    ],
    enabled:
      accounting.observations &&
      section === 'channels' &&
      !!query.data &&
      (channelSort.sort !== 'balance' || channelSort.unit.trim() !== ''),
    queryFn: async ({ signal }) => {
      const snapshotDigest =
        snapshotAnchor.current?.selection === selection
          ? snapshotAnchor.current.digest
          : undefined
      if (page > 1 && !snapshotDigest) {
        throw new Error('Refresh from the first page')
      }
      const current = query.data
      if (!current) throw new Error('Routing configuration is unavailable')
      const result = await getNativeChannelPage(
        scope,
        current.version,
        { ...channelSort, ...channelFilters, page, search, snapshotDigest },
        signal
      )
      if (!signal.aborted) {
        const anchor = { selection, digest: result.snapshot_digest }
        snapshotAnchor.current = anchor
        setChannelListing({ anchor, balanceUnits: result.balance_units })
      }
      return result
    },
    retry: false,
  })
  const verifiedChannelPage =
    sortedChannels.isSuccess &&
    sortedChannels.isFetchedAfterMount &&
    !sortedChannels.isFetching &&
    channelListing.anchor?.selection === selection &&
    channelListing.anchor.digest === sortedChannels.data.snapshot_digest
      ? sortedChannels.data
      : undefined
  const serverChannels = accounting.observations && section === 'channels'

  function report(cause: unknown) {
    setError(
      nativeTimeoutErrorMessage(cause, t) ??
        getServerErrorMessage(cause) ??
        (cause instanceof Error ? cause.message : t('Failed to save'))
    )
  }
  function changeEditor(destination: EditorDestination) {
    if (editorDestination && 'editor' in destination) {
      // The replacement form focuses its first field when it opens.
      discardFocus.current = false
    }
    if ('close' in destination && editor?.trigger) {
      if (editor.channelId) discardFocus.current = false
      else if (editorDestination) discardFocus.current = editor.trigger
      else editor.trigger.focus({ preventScroll: true })
    }
    const next = 'editor' in destination ? destination.editor : null
    if (next?.channelId) channelEditorTrigger.current = next.trigger ?? null
    setModelAfterSave(null)
    setEditor(next)
    editorBaseline.current = next?.document ?? ''
    if ('section' in destination) {
      setSection(destination.section)
      setPage(1)
    }
    setEditorDestination(null)
    setPreview(null)
    setSecret('')
    setError(null)
    setConfirmation(null)
    setResult(null)
  }
  function requestEditorChange(destination: EditorDestination) {
    discardFocus.current = null
    if (editor && (editor.document !== editorBaseline.current || secret)) {
      setEditorDestination(destination)
    } else {
      changeEditor(destination)
    }
  }
  function openEditor(value: Omit<Editor, 'version'>) {
    if (!configuration) return
    requestEditorChange({
      editor: { ...value, version: { ...configuration.version } },
    })
  }
  async function refresh() {
    snapshotAnchor.current = null
    setChannelListing((listing) => ({ ...listing, anchor: null }))
    setPage(1)
    setListingRevision((revision) => revision + 1)
    await Promise.all([
      query.refetch(),
      client.invalidateQueries({
        queryKey: [
          'native-channel-statistics',
          scope.userId,
          scope.sessionId,
          scope.role,
        ],
      }),
      client.invalidateQueries({ queryKey: ['channels'] }),
      client.invalidateQueries({ queryKey: ['pricing'] }),
    ])
  }
  async function run(
    operation: () => Promise<unknown>,
    close = false
  ): Promise<boolean> {
    if (!configurationWritable || busy || operationPending.current) return false
    operationPending.current = true
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const value = await operation()
      setResult(value)
      toast.success(t('Operation successful'))
      setConfirmation(null)
      if (close) {
        setEditor(null)
        setPreview(null)
        setSecret('')
      }
      await refresh()
      return true
    } catch (cause) {
      report(cause)
      setPreview(null)
      await refresh()
      return false
    } finally {
      operationPending.current = false
      setBusy(false)
    }
  }
  async function submitCopy() {
    if (
      !configurationWritable ||
      !copyChannel ||
      !configuration ||
      !secretsAllowed ||
      (!copyReset && !accounting.preserveCounters) ||
      !configuration.secret_storage_enabled ||
      busy ||
      copyPending.current
    ) {
      return
    }
    copyPending.current = true
    setBusy(true)
    setCopyError(null)
    setResult(null)
    try {
      const receipt = await copyNativeChannel(
        scope,
        copyChannel.id,
        configuration.version,
        copySuffix,
        copyReset
      )
      setResult(receipt)
      toast.success(t('Channel copied successfully'))
      setCopyChannel(null)
      setCopySuffix('_copy')
      setCopyReset(true)
    } catch (cause) {
      setCopyError(
        getServerErrorMessage(cause) ??
          (cause instanceof Error ? cause.message : t('Failed to copy channel'))
      )
    } finally {
      await refresh()
      copyPending.current = false
      setBusy(false)
    }
  }
  function channelEditor(
    channel?: NativeUpstream,
    trigger?: HTMLButtonElement,
    fetchCatalogOnOpen = false
  ) {
    if (!configuration) return
    const id =
      channel?.id ??
      nextId([
        ...configuration.routing.upstreams.map((item) => item.id),
        configuration.channel_id_high_water ?? 0,
      ])
    const attachments = configuration.routing.routes.flatMap((route) =>
      route.targets
        .filter((target) => target.upstream_id === id)
        .map(
          ({
            id: _targetId,
            upstream_id: _upstreamId,
            enabled: _enabled,
            client_protocols: _clientProtocols,
            ...target
          }) => ({ route_id: route.id, ...target })
        )
    )
    openEditor({
      title: channel ? t('Edit Channel') : t('Create Channel'),
      method: channel ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/channels${channel ? `/${id}` : ''}`,
      secret: !channel,
      channelId: id,
      fetchCatalogOnOpen,
      trigger,
      document: pretty({
        expected_version: configuration.version,
        id,
        name: configuration.bindings.names[id] ?? '',
        tag: configuration.bindings.tags?.[id] ?? '',
        provider: configuration.bindings.channel_providers?.[id],
        note: configuration.bindings.channel_notes?.[id],
        endpoint: channel?.endpoint ?? '',
        protocol: channel?.protocol ?? 'openai_responses',
        enabled: channel?.enabled ?? false,
        contexts: configuration.bindings.contexts.filter(
          (context) => context.upstream_id === id
        ),
        attachments,
        ...configuration.bindings.channel_target_defaults?.[id],
        groups: configuration.bindings.channel_groups?.[id] ?? [
          ...new Set(
            configuration.routing.routes
              .filter((route) =>
                attachments.some((target) => target.route_id === route.id)
              )
              .map((route) => route.group)
          ),
        ],
      }),
    })
  }
  function vendorEditor(vendor?: NativeVendor) {
    if (!configuration) return
    openEditor({
      title: vendor ? t('Edit') : t('Create'),
      method: vendor ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/vendors${vendor ? `/${vendor.id}` : ''}`,
      document: pretty({
        expected_version: configuration.version,
        vendor: vendor ?? {
          id: nextId(
            Object.values(configuration.bindings.vendors).map((item) => item.id)
          ),
          name: '',
          description: '',
        },
      }),
    })
  }
  function modelEditor(
    route?: NativeModelRoute,
    discoveredModel = '',
    discoveredChannel?: number,
    source = configuration,
    requestedGroup?: string,
    afterChannelSave = false
  ) {
    if (!source) return
    const id = route?.id ?? nextId(source.routing.routes.map((item) => item.id))
    const group =
      route?.group ??
      requestedGroup ??
      source.routing.groups[0]?.name ??
      'default'
    const model = route?.model ?? discoveredModel
    const next = {
      title: route ? t('Edit') : t('Add Model'),
      method: route ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/models${route ? `/${id}` : ''}`,
      previewPath: `${NATIVE_ROOT}/models/preview`,
      modelEditing: !!route,
      document: pretty({
        expected_version: source.version,
        route: route ?? {
          id,
          model,
          group,
          enabled: false,
          speeds: ['standard'],
          targets: [],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        },
        metadata: source.bindings.models[id] ?? {
          display_name: defaultModelDisplayName(model),
          description: '',
          vendor_id: null,
        },
        prices: [],
        contexts: source.bindings.contexts.filter((context) =>
          route?.targets.some(
            (target) =>
              target.upstream_id === context.upstream_id &&
              target.upstream_model === context.upstream_model
          )
        ),
      }),
    } satisfies Omit<Editor, 'version'>
    if (discoveredChannel !== undefined) {
      const draft = readNativeModelDraft(next.document)
      if (draft) {
        next.document = pretty(
          replaceModelTargets(
            draft,
            [newModelTarget(draft, source, discoveredChannel)],
            source
          )
        )
      }
    }
    if (afterChannelSave) {
      changeEditor({ editor: { ...next, version: { ...source.version } } })
      return
    }
    openEditor(next)
  }
  function priceEditor() {
    if (!configuration) return
    openEditor({
      title: t('Pricing'),
      method: 'PUT',
      path: `${NATIVE_ROOT}/prices`,
      previewPath: `${NATIVE_ROOT}/prices/preview`,
      document: pretty({
        expected_version: configuration.version,
        prices: {
          ...configuration.prices,
          version: configuration.version.pricing + 1,
        },
      }),
    })
  }
  function draftEditor(draft?: NativePriceDraft) {
    if (!configuration) return
    const id =
      draft?.id ??
      nextId(
        Object.values(configuration.bindings.price_drafts).map(
          (item) => item.id
        )
      )
    openEditor({
      title: t('Save draft'),
      method: 'PUT',
      path: `${NATIVE_ROOT}/prices/drafts/${id}`,
      document: pretty({
        expected_version: configuration.version,
        name: draft?.name ?? '',
        prices: draft?.prices ?? {
          ...configuration.prices,
          version: configuration.version.pricing + 1,
        },
      }),
    })
  }
  function reloadEditor() {
    if (!editor || !configuration) return
    const id = Number(editor.path.split('/').at(-1))
    if (editor.channelId) {
      const channel = configuration.routing.upstreams.find(
        (item) => item.id === id
      )
      if (editor.method === 'POST' || channel) {
        channelEditor(channel, editor.trigger)
        return
      }
    } else if (editor.modelEditing !== undefined) {
      const route = configuration.routing.routes.find((item) => item.id === id)
      if (!editor.modelEditing || route) {
        modelEditor(route)
        return
      }
    }
    setError(
      t('This item was removed. Close the editor and select another item.')
    )
  }
  function editorPayload(editor: Editor) {
    const body = parseNativeDocument(editor.document)
    if (!configuration) return body
    if (
      editor.channelId &&
      (!Array.isArray(body.groups) || body.groups.length === 0)
    ) {
      throw new Error(t('Group is required'))
    }
    assertNativeDraftTimeouts(body, configuration)
    if (editor.modelEditing === undefined) return body

    const model = readNativeModelDraft(editor.document)
    if (!model) throw new Error(t('Invalid JSON'))
    const { id, group } = model.route
    const name = model.route.model
    const official = configuration.bindings.official_prices?.models ?? {}
    if (Object.hasOwn(official, name)) return { ...body, prices: [] }

    const previous = configuration.routing.routes.find(
      (route) => route.id === id
    )
    if (!previous || previous.model !== name || previous.group !== group) {
      throw new Error(t('Publish the official base price first.'))
    }
    return {
      ...body,
      prices: configuration.prices.rules.filter(
        (rule) => rule.model === name && rule.group === group
      ),
    }
  }
  async function previewEditor() {
    if (!configurationWritable || !editor?.previewPath || busy || editorStale) {
      return
    }
    setBusy(true)
    setError(null)
    try {
      const body = editorPayload(editor)
      const request =
        editor.modelEditing === undefined
          ? body
          : { editing: editor.modelEditing, model: body }
      const value = await nativeRequest<{
        confirmation_digest: string
        [key: string]: unknown
      }>('POST', editor.previewPath, request)
      if (
        typeof value.confirmation_digest !== 'string' ||
        !/^[0-9a-f]{64}$/.test(value.confirmation_digest)
      ) {
        throw new Error('The preview did not return a confirmation digest.')
      }
      setPreview(value)
    } catch (cause) {
      report(cause)
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }
  function saveEditor() {
    if (!editor || editorStale || (editor.previewPath && !preview)) return
    void run(async () => {
      const body = editorPayload(editor)
      if (editor.secret) body.secret = secret
      if (preview) body.confirmation_digest = preview.confirmation_digest
      return nativeRequest(editor.method, editor.path, body)
    }, true).then((saved) => {
      if (!saved || !editor.channelId || !modelAfterSave) return
      const current = client.getQueryState<NativeConfiguration>(queryKey)
      const latest = current?.status === 'success' ? current.data : undefined
      if (
        !latest ||
        !latest.routing.groups.some(
          (item) => item.name === modelAfterSave.group
        )
      ) {
        setError(
          t(
            'The channel was saved. Reload the model editor to select a current group.'
          )
        )
        setModelAfterSave(null)
        return
      }
      modelEditor(
        undefined,
        modelAfterSave.model,
        editor.channelId,
        latest,
        modelAfterSave.group,
        true
      )
    })
  }
  function remove(path: string) {
    if (!configurationWritable || !configuration) return
    if (confirmation !== path) {
      setConfirmation(path)
      return
    }
    void run(() =>
      nativeRequest('DELETE', path, {
        expected_version: configuration.version,
      })
    )
  }
  if (query.isLoading) return <LoadingState />
  if (query.isError || !configuration) {
    return (
      <ErrorState
        description={getServerErrorMessage(query.error) ?? t('Failed to load')}
        onRetry={() => void query.refetch()}
      />
    )
  }

  const matches = (...values: Array<string | number>) =>
    values.some((value) =>
      String(value).toLowerCase().includes(search.trim().toLowerCase())
    )
  const enabledGroups = new Set(
    configuration.routing.groups
      .filter((group) => group.enabled)
      .map((group) => group.name)
  )
  const activeRoutes = configuration.routing.routes.filter(
    (route) => route.enabled && enabledGroups.has(route.group)
  )
  const localUpstreams = serverChannels
    ? []
    : configuration.routing.upstreams.filter((channel) => {
        if (
          channelFilters.status !== 'all' &&
          channel.enabled !== (channelFilters.status === 'enabled')
        ) {
          return false
        }
        if (
          channelFilters.protocol &&
          channel.protocol !== channelFilters.protocol
        ) {
          return false
        }
        const routes = activeRoutes.filter((route) =>
          route.targets.some(
            (target) => target.upstream_id === channel.id && target.enabled
          )
        )
        return (
          (!channelFilters.group ||
            routes.some((route) => route.group === channelFilters.group)) &&
          matches(
            channel.id,
            configuration.bindings.names[channel.id] ?? '',
            configuration.bindings.tags?.[channel.id] ?? '',
            channel.endpoint,
            ...routes.map((route) => route.model)
          )
        )
      })
  const channelFiltersActive =
    channelFilters.status !== 'all' ||
    channelFilters.group !== '' ||
    channelFilters.protocol !== '' ||
    search.trim() !== ''
  function clearChannelFilters() {
    updateChannelSelection([])
    setChannelFilters({ status: 'all', group: '', protocol: '' })
    setSearch('')
    setPage(1)
  }
  const upstreamById = new Map(
    configuration.routing.upstreams.map((channel) => [channel.id, channel])
  )
  const listedItems = verifiedChannelPage?.items ?? []
  const upstreams = serverChannels
    ? listedItems.flatMap((item) => {
        const channel = upstreamById.get(item.id)
        return channel ? [channel] : []
      })
    : localUpstreams
  const statisticsById = new Map(
    verifiedChannelPage?.items.map((item) => [item.id, item]) ?? []
  )
  const models = configuration.routing.routes.filter((route) =>
    matches(
      route.id,
      route.model,
      route.group,
      configuration.bindings.models[route.id]?.display_name ?? ''
    )
  )
  const vendors = Object.values(configuration.bindings.vendors).filter(
    (vendor) => matches(vendor.id, vendor.name)
  )
  const prices = configuration.prices.rules.filter((price) =>
    matches(
      String(price.id),
      String(price.model),
      String(price.group),
      String(price.speed)
    )
  )
  const priceModels = [...new Set(prices.map((rule) => String(rule.model)))]
  const unpriced =
    section === 'unpriced'
      ? configuration.routing.upstreams
          .flatMap((channel) => {
            const name =
              configuration.bindings.names[channel.id] ??
              t('Channel {{id}}', { id: channel.id })
            return (
              configuration.bindings.discoveries[channel.id]?.pending_models ??
              []
            )
              .filter((model) => matches(model, name, channel.id))
              .map((model) => ({ channel, name, model }))
          })
          .sort(
            (a, b) =>
              a.model.localeCompare(b.model) || a.channel.id - b.channel.id
          )
      : []
  const total =
    section === 'channels'
      ? serverChannels
        ? (verifiedChannelPage?.total ?? 0)
        : upstreams.length
      : section === 'models'
        ? models.length
        : section === 'vendors'
          ? vendors.length
          : section === 'unpriced'
            ? unpriced.length
            : priceModels.length
  const paginationKnown = !serverChannels || Boolean(verifiedChannelPage)
  const lastPage = Math.max(1, Math.ceil(total / 25))
  // Local lists can shrink while open; server channel pages are validated by digest.
  const shownPage =
    section === 'channels' && serverChannels ? page : Math.min(page, lastPage)
  const start = (shownPage - 1) * 25
  const visiblePriceModels = new Set(priceModels.slice(start, start + 25))
  const visibleChannels = serverChannels
    ? upstreams
    : upstreams.slice(start, start + 25)
  const visibleChannelIds = visibleChannels.map((channel) => channel.id)
  const selectedVisibleIds = visibleChannelIds.filter((id) =>
    selectedChannelIds.includes(id)
  )
  const allVisibleSelected =
    visibleChannelIds.length > 0 &&
    selectedVisibleIds.length === visibleChannelIds.length
  const batchRoutingSnapshot = JSON.stringify([
    channelSelectionSnapshot,
    selectedVisibleIds,
  ])
  const batchRoutingOpen =
    selectedVisibleIds.length > 0 && batchRoutingEditor === batchRoutingSnapshot
  const batchTagValue =
    selectedVisibleIds.length > 0 &&
    batchTagEditor?.snapshot === channelSelectionSnapshot
      ? batchTagEditor.value
      : null
  async function changeSelectedStatus(enabled: boolean) {
    if (!configuration || selectedVisibleIds.length === 0) return
    const saved = await run(() =>
      setNativeChannelsEnabled(
        selectedVisibleIds,
        enabled,
        configuration.version,
        scope
      )
    )
    if (saved) updateChannelSelection([])
  }
  async function saveSelectedTag(tag: string | null) {
    if (!configuration || selectedVisibleIds.length === 0) return
    const saved = await run(() =>
      setNativeChannelsTag(
        selectedVisibleIds,
        tag,
        configuration.version,
        scope
      )
    )
    if (saved) updateChannelSelection([])
  }
  async function saveSelectedRouting(patch: NativeChannelBatchPatch) {
    if (!configuration || selectedVisibleIds.length === 0) return
    const saved = await run(() =>
      updateNativeChannels(
        scope,
        configuration.version,
        selectedVisibleIds,
        patch
      )
    )
    if (saved) {
      updateChannelSelection([])
      setBatchRoutingEditor(null)
    }
  }
  function deleteButton(path: string) {
    return (
      <Button
        type='button'
        variant={confirmation === path ? 'destructive' : 'outline'}
        className='min-h-11'
        disabled={writeDisabled}
        onClick={() => remove(path)}
      >
        {confirmation === path ? t('Confirm delete') : t('Delete')}
      </Button>
    )
  }
  const editorConfigurationField = editor && (
    <label className='block space-y-2'>
      <span className='text-sm'>{t('Configuration')}</span>
      <Textarea
        className='min-h-72 font-mono text-xs'
        value={editor.document}
        readOnly={editor.readOnly}
        disabled={busy}
        onChange={(event) => {
          setEditor({ ...editor, document: event.target.value })
          setPreview(null)
        }}
      />
    </label>
  )
  const editorForm = editor && (
    <form
      className={
        editor.channelId
          ? 'flex min-h-0 min-w-0 flex-1 flex-col'
          : 'min-w-0 p-5 sm:p-7'
      }
      aria-label={editor.title}
      ref={editorRef}
      onInvalidCapture={(event) => {
        if (!editor.channelId || !(event.target instanceof HTMLElement)) {
          return
        }
        let disclosure = event.target.closest('details')
        while (disclosure && event.currentTarget.contains(disclosure)) {
          if (!disclosure.open) disclosure.open = true
          disclosure = disclosure.parentElement?.closest('details') ?? null
        }
      }}
      onSubmit={(event) => {
        event.preventDefault()
        if (editor.previewPath && !preview) void previewEditor()
        else saveEditor()
      }}
    >
      <div
        className={
          editor.channelId
            ? 'min-h-0 flex-1 space-y-6 overflow-y-auto px-5 pb-5 sm:px-7'
            : 'space-y-6'
        }
      >
        {!editor.channelId && <h3 className='font-medium'>{editor.title}</h3>}
        {editor.channelId && error && (
          <Alert variant='destructive' role='alert'>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        {editorStale && (
          <Alert role='status'>
            <AlertDescription className='space-y-3'>
              <p>
                {t(
                  'Configuration changed. Your draft is preserved. Reload the editor before saving.'
                )}
              </p>
              <Button
                type='button'
                variant='outline'
                className='min-h-11'
                disabled={busy}
                onClick={reloadEditor}
              >
                {t('Reload editor')}
              </Button>
            </AlertDescription>
          </Alert>
        )}
        {editor.channelId && modelAfterSave && (
          <Alert role='status'>
            <AlertDescription className='flex flex-wrap items-center justify-between gap-2'>
              <span>
                {t(
                  'Save this channel to continue creating {{model}} in {{group}}.',
                  modelAfterSave
                )}
              </span>
              <Button
                type='button'
                variant='ghost'
                size='sm'
                disabled={busy}
                onClick={() => setModelAfterSave(null)}
              >
                {t('Clear')}
              </Button>
            </AlertDescription>
          </Alert>
        )}
        {editor.channelId && (
          <NativeChannelFields
            key={editor.path}
            document={editor.document}
            configuration={configuration}
            disabled={busy}
            fetchCatalogOnOpen={editor.fetchCatalogOnOpen}
            fetchModels={
              editor.method === 'PUT' && accounting.observations
                ? () =>
                    nativeRequest<string[]>(
                      'GET',
                      `/api/channel/fetch_models/${editor.channelId}`
                    )
                : undefined
            }
            onCreateModel={(model, group) => {
              if (editor.document !== editorBaseline.current) {
                setModelAfterSave({ model, group })
              } else {
                modelEditor(
                  undefined,
                  model,
                  editor.channelId,
                  configuration,
                  group
                )
              }
            }}
            credential={
              editor.secret ? (
                <label className='block space-y-2'>
                  <span className='text-sm font-medium'>{t('API key')}</span>
                  <Input
                    type='password'
                    autoComplete='off'
                    className='min-h-11'
                    value={secret}
                    disabled={busy}
                    required
                    onChange={(event) => setSecret(event.target.value)}
                  />
                </label>
              ) : (
                <div className='flex min-w-0 items-end'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={
                      writeDisabled ||
                      editorStale ||
                      !secretsAllowed ||
                      !configuration.secret_storage_enabled
                    }
                    onClick={() => setKeyChannel(editor.channelId ?? null)}
                  >
                    <KeyRound className='size-4' />
                    {t('API Keys')}
                  </Button>
                </div>
              )
            }
            onChange={(document) => {
              setEditor({ ...editor, document })
              setPreview(null)
            }}
          />
        )}
        {editor.modelEditing !== undefined && (
          <NativeModelFields
            document={editor.document}
            configuration={configuration}
            disabled={busy}
            onChange={(document) => {
              setEditor({ ...editor, document })
              setPreview(null)
            }}
          />
        )}
        {editor.channelId ? (
          <details className='group/advanced border-t'>
            <summary className='flex min-h-12 cursor-pointer items-center justify-between gap-3 text-sm font-medium'>
              {t('Advanced Settings')}
              <ChevronDown
                aria-hidden='true'
                className='text-muted-foreground size-4 shrink-0 group-open/advanced:rotate-180'
              />
            </summary>
            <div className='mb-4'>
              <Button
                type='button'
                variant='outline'
                disabled={
                  writeDisabled ||
                  editorStale ||
                  editor.document !== editorBaseline.current
                }
                onClick={() => {
                  const id = editor.channelId
                  if (id === undefined) return
                  changeEditor({ close: true })
                  setOverrideId(id)
                }}
              >
                {t('Override Rules')}
              </Button>
            </div>
            <div className='mb-4 max-w-md'>
              <NativeChannelTagField
                document={editor.document}
                disabled={busy}
                onChange={(document) => setEditor({ ...editor, document })}
              />
            </div>
            <details className='border-t'>
              <summary className='min-h-11 cursor-pointer py-3 text-sm'>
                {t('Raw JSON Configuration')}
              </summary>
              {editorConfigurationField}
            </details>
          </details>
        ) : (
          <details open={editor.modelEditing === undefined}>
            <summary className='cursor-pointer py-2 text-sm font-medium'>
              {t('Advanced Settings')}
            </summary>
            {editorConfigurationField}
          </details>
        )}
        {preview && !editorStale && (
          <pre
            className='max-h-80 overflow-auto border p-3 text-xs'
            aria-label={t('Preview')}
          >
            {pretty(preview)}
          </pre>
        )}
      </div>
      <div
        className={
          editor.channelId
            ? 'border-border bg-popover flex shrink-0 justify-end gap-2 border-t px-5 py-4 pb-[max(1rem,env(safe-area-inset-bottom))] sm:px-7'
            : 'mt-6 flex flex-wrap gap-2'
        }
      >
        <Button
          type='submit'
          className='min-h-11'
          disabled={writeDisabled || editorStale}
        >
          {editor.previewPath
            ? preview
              ? t('Confirm and apply')
              : t('Preview')
            : t('Save')}
        </Button>
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={busy}
          onClick={() => requestEditorChange({ close: true })}
        >
          {t('Cancel')}
        </Button>
      </div>
    </form>
  )

  return (
    <section
      className='@container/routing min-w-0 space-y-6'
      aria-label={t('Configuration')}
    >
      <div className='flex min-w-0 flex-wrap items-center justify-between gap-3'>
        <nav
          className='flex min-w-0 flex-1 items-center gap-3 overflow-x-auto'
          aria-label={t('Configuration')}
        >
          {sections.map(([value, label]) => (
            <Button
              key={value}
              type='button'
              variant='ghost'
              disabled={busy}
              aria-current={section === value ? 'page' : undefined}
              className='text-muted-foreground aria-[current=page]:text-foreground aria-[current=page]:after:bg-foreground relative min-h-11 shrink-0 rounded-none bg-transparent px-1 shadow-none after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 hover:bg-transparent'
              onClick={() => {
                if (section !== value) requestEditorChange({ section: value })
              }}
            >
              {t(label)}
            </Button>
          ))}
        </nav>
        <Button
          type='button'
          variant='ghost'
          className='min-h-11 shrink-0'
          disabled={busy || query.isFetching}
          onClick={() => void refresh()}
        >
          <RefreshCw className='size-4' />
          {t('Refresh')}
        </Button>
      </div>
      <Sheet
        open={Boolean(editor?.channelId)}
        onOpenChange={(open) => {
          if (!open && !busy && editor?.channelId) {
            requestEditorChange({ close: true })
          }
        }}
      >
        <SheetContent
          className='@container/routing h-dvh w-full gap-0 sm:max-w-3xl'
          finalFocus={() => channelEditorTrigger.current ?? false}
          initialFocus={() =>
            editorRef.current?.querySelector<HTMLInputElement>('input') ?? false
          }
        >
          <SheetHeader className='shrink-0 pr-14'>
            <SheetTitle>{editor?.title}</SheetTitle>
          </SheetHeader>
          {editor?.channelId && editorForm}
        </SheetContent>
      </Sheet>
      {!editor?.channelId && editorForm}
      <Dialog
        open={editorDestination !== null}
        finalFocus={() => discardFocus.current}
        onOpenChange={(open) => {
          if (!open) setEditorDestination(null)
        }}
        title={t('You have unsaved changes')}
        description={t(
          'You have unsaved changes. Are you sure you want to leave?'
        )}
        footer={
          <>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              onClick={() => setEditorDestination(null)}
            >
              {t('Stay')}
            </Button>
            <Button
              type='button'
              variant='destructive'
              className='min-h-11'
              onClick={() => {
                if (editorDestination) changeEditor(editorDestination)
              }}
            >
              {t('Leave')}
            </Button>
          </>
        }
      >
        {null}
      </Dialog>
      <div className='flex min-w-0 flex-wrap items-end gap-3'>
        <label className='min-w-0 flex-1 basis-56 space-y-2'>
          <span className='text-sm'>{t('Search')}</span>
          <Input
            aria-label={t('Search')}
            className='min-h-11'
            placeholder={
              section === 'channels'
                ? [t('Name'), t('ID'), t('Models'), t('Tag')].join(' / ')
                : undefined
            }
            value={search}
            onChange={(event) => {
              setSearch(event.target.value)
              setPage(1)
            }}
          />
        </label>
        {section === 'channels' && (
          <>
            <div className='min-w-0 flex-1 basis-64 @min-[40rem]/routing:basis-[32rem]'>
              <NativeChannelFilterFields
                value={channelFilters}
                groups={[...enabledGroups]}
                disabled={busy}
                onChange={(value) => {
                  updateChannelSelection([])
                  setChannelFilters(value)
                  setPage(1)
                }}
              />
            </div>
            <Button
              type='button'
              className='min-h-11 shrink-0'
              disabled={
                writeDisabled ||
                !secretsAllowed ||
                !configuration.secret_storage_enabled
              }
              onClick={(event) => channelEditor(undefined, event.currentTarget)}
            >
              <Plus className='size-4' />
              {t('Create Channel')}
            </Button>
            {channelFiltersActive && (
              <Button
                type='button'
                variant='ghost'
                className='min-h-11 shrink-0'
                disabled={busy}
                onClick={clearChannelFilters}
              >
                {t('Clear filters')}
              </Button>
            )}
          </>
        )}
      </div>
      {error && !editor?.channelId && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {section === 'channels' && (
        <>
          {(!secretsAllowed || !configuration.secret_storage_enabled) && (
            <p className='text-muted-foreground text-sm' role='status'>
              {t('API Keys')}: {t('Capability unavailable')}
            </p>
          )}
          {serverChannels && (
            <>
              <NativeChannelSortFields
                disabled={sortedChannels.isFetching}
                units={channelListing.balanceUnits}
                value={{ ...channelSort, page, search }}
                onChange={(value) => {
                  setChannelSort({
                    sort: value.sort,
                    order: value.order,
                    unit: value.unit,
                  })
                  setPage(1)
                }}
              />
              {channelSort.sort === 'balance' &&
              channelSort.unit.trim() === '' ? (
                <p role='status'>{t('channelSorting.balanceUnit')}</p>
              ) : sortedChannels.isFetching ? (
                <LoadingState />
              ) : null}
              {sortedChannels.isError && (
                <ErrorState
                  description={t(
                    isChannelSnapshotChanged(sortedChannels.error)
                      ? 'channelSorting.snapshotChanged'
                      : 'channelUsage.error'
                  )}
                  onRetry={() => void refresh()}
                />
              )}
            </>
          )}
          {verifiedChannelPage?.summary && (
            <NativeChannelOperationsSummary
              value={verifiedChannelPage.summary}
            />
          )}
          <div className='flex flex-wrap items-center gap-3'>
            <div className='text-muted-foreground flex min-h-11 items-center gap-2 text-sm'>
              <Checkbox
                aria-label={t('Select current page')}
                checked={allVisibleSelected}
                indeterminate={
                  selectedVisibleIds.length > 0 && !allVisibleSelected
                }
                disabled={busy || visibleChannels.length === 0}
                onCheckedChange={(checked) =>
                  updateChannelSelection(checked ? visibleChannelIds : [])
                }
              />
              <span>{t('Select current page')}</span>
            </div>
            {selectedVisibleIds.length > 0 && (
              <div
                className='flex flex-wrap items-center gap-2'
                data-slot='channel-batch-actions'
              >
                <span className='text-sm font-medium'>
                  {t('{{count}} selected', {
                    count: selectedVisibleIds.length,
                  })}
                </span>
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={writeDisabled || !accounting.balanceProbe}
                  onClick={() =>
                    setBalanceBatchChannels(
                      selectedVisibleIds.map((id) => ({
                        id,
                        name:
                          configuration.bindings.names[id] ??
                          t('Channel {{id}}', { id }),
                      }))
                    )
                  }
                >
                  {t('Refresh selected balances')}
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={writeDisabled}
                  onClick={() => void changeSelectedStatus(true)}
                >
                  {t('Enable')}
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={writeDisabled}
                  onClick={() => void changeSelectedStatus(false)}
                >
                  {t('Disable')}
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={busy}
                  onClick={() => {
                    setBatchRoutingEditor(null)
                    setBatchTagEditor({
                      snapshot: channelSelectionSnapshot,
                      value: batchTagValue ?? '',
                    })
                  }}
                >
                  {t('Tag')}
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  disabled={busy}
                  onClick={() => {
                    setBatchTagEditor(null)
                    setBatchRoutingEditor(batchRoutingSnapshot)
                  }}
                >
                  {t('Edit routing')}
                </Button>
              </div>
            )}
            <div className='bg-muted ms-auto hidden shrink-0 items-center rounded-lg p-0.5 @min-[40rem]/routing:flex'>
              {(
                [
                  ['cards', LayoutGrid, t('Card view')],
                  ['list', List, t('List view')],
                ] as const
              ).map(([layout, Icon, label]) => (
                <Button
                  key={layout}
                  type='button'
                  size='icon'
                  variant={channelLayout === layout ? 'secondary' : 'ghost'}
                  className='aria-pressed:bg-background aria-pressed:text-foreground size-11 sm:size-9'
                  aria-label={label}
                  aria-pressed={channelLayout === layout}
                  title={label}
                  onClick={() => setChannelLayout(layout)}
                >
                  <Icon className='size-4' />
                </Button>
              ))}
            </div>
          </div>
          {batchRoutingOpen && (
            <NativeChannelBatchEditor
              key={batchRoutingSnapshot}
              selectedIds={selectedVisibleIds}
              configuration={configuration}
              enforceMinimum={configuration.routing.upstreams.some(
                (item) => item.enabled && selectedVisibleIds.includes(item.id)
              )}
              busy={busy}
              writable={configurationWritable}
              onSave={saveSelectedRouting}
              onCancel={() => setBatchRoutingEditor(null)}
            />
          )}
          {batchTagValue !== null && (
            <form
              className='bg-card grid grid-cols-3 items-end gap-2 rounded-lg p-4 sm:grid-cols-[minmax(0,1fr)_repeat(3,auto)] sm:gap-3'
              aria-label={t('Set tag for selected channels')}
              onSubmit={(event) => {
                event.preventDefault()
                if (batchTagValue.trim()) void saveSelectedTag(batchTagValue)
              }}
            >
              <label className='col-span-3 grid min-w-0 gap-2 text-sm sm:col-span-1'>
                <span>{t('Set tag for selected channels')}</span>
                <Input
                  className='border-muted-foreground/60 bg-background! min-h-11'
                  value={batchTagValue}
                  maxLength={128}
                  required
                  disabled={busy}
                  onChange={(event) =>
                    setBatchTagEditor({
                      snapshot: channelSelectionSnapshot,
                      value: event.target.value,
                    })
                  }
                />
              </label>
              <Button
                type='submit'
                className='min-h-11'
                disabled={writeDisabled || !batchTagValue.trim()}
              >
                {t('Apply')}
              </Button>
              <Button
                type='button'
                variant='outline'
                className='min-h-11'
                disabled={writeDisabled}
                onClick={() => void saveSelectedTag(null)}
              >
                {t('Clear')}
              </Button>
              <Button
                type='button'
                variant='ghost'
                className='min-h-11'
                disabled={busy}
                onClick={() => setBatchTagEditor(null)}
              >
                {t('Cancel')}
              </Button>
            </form>
          )}
          {visibleChannels.length === 0 &&
            (!serverChannels || verifiedChannelPage) && (
              <div
                className='bg-card space-y-3 rounded-xl px-6 py-14 text-center'
                role='status'
              >
                <h3 className='font-medium'>
                  {channelFiltersActive
                    ? t('No results found')
                    : t('No channels found')}
                </h3>
                <p className='text-muted-foreground text-sm'>
                  {channelFiltersActive
                    ? t('Try a different search or clear the current filter.')
                    : t('Connect a provider to start routing model requests.')}
                </p>
                {channelFiltersActive && (
                  <Button
                    type='button'
                    variant='outline'
                    onClick={clearChannelFilters}
                  >
                    {t('Clear')}
                  </Button>
                )}
              </div>
            )}
          <div data-layout={channelLayout} className='native-channel-list'>
            {visibleChannels.length > 0 && (
              <div
                aria-hidden='true'
                className='native-channel-list-heading text-muted-foreground px-4 py-2 text-xs font-medium'
              >
                <span>{t('Name')}</span>
                <span>{t('Status')}</span>
                <span>{t('Models')}</span>
                <span>{t('Priority')}</span>
                <span>{t('Weight')}</span>
                <span>{t('Actions')}</span>
              </div>
            )}
            {visibleChannels.map((channel) => {
              const provider = resolveNativeProvider(
                configuration,
                configuration.bindings.channel_providers?.[channel.id],
                channel.protocol,
                channel.endpoint
              )

              const name =
                configuration.bindings.names[channel.id] ??
                t('Channel {{id}}', { id: channel.id })
              const routes = configuration.routing.routes.filter((route) =>
                route.targets.some(
                  (target) => target.upstream_id === channel.id
                )
              )
              const groups = [
                ...new Set(routes.map((route) => route.group)),
              ].sort()
              const markedGroups =
                configuration.bindings.channel_groups?.[channel.id]
              const assignment =
                groupEditor?.id === channel.id ? groupEditor : null
              const targets = routes.flatMap((route) =>
                route.targets.filter(
                  (target) => target.upstream_id === channel.id
                )
              )
              const routingMetrics = (['priority', 'weight'] as const).map(
                (key) => ({
                  label: key === 'priority' ? t('Priority') : t('Weight'),
                  value: [...new Set(targets.map((target) => target[key]))]
                    .sort((a, b) => a - b)
                    .join(', '),
                })
              )
              const discovery = configuration.bindings.discoveries[channel.id]
              return (
                <article
                  key={channel.id}
                  className='bg-card min-w-0 space-y-3 rounded-xl p-4'
                  aria-label={name}
                >
                  <div className='native-channel-overview'>
                    <div className='flex min-w-0 items-start gap-2.5'>
                      <Checkbox
                        className='mt-1'
                        aria-label={name}
                        checked={selectedVisibleIds.includes(channel.id)}
                        disabled={busy}
                        onCheckedChange={(checked) =>
                          updateChannelSelection(
                            checked
                              ? [...selectedChannelIds, channel.id]
                              : selectedChannelIds.filter(
                                  (id) => id !== channel.id
                                )
                          )
                        }
                      />
                      <div className='min-w-0 flex-1 space-y-1.5'>
                        <div className='flex flex-wrap items-baseline gap-x-3 gap-y-1'>
                          <h3 className='text-base leading-snug font-semibold tracking-tight [overflow-wrap:anywhere]'>
                            {name}
                          </h3>
                          <p className='text-muted-foreground text-xs tabular-nums'>
                            #{channel.id}
                          </p>
                        </div>
                        <p
                          className='text-muted-foreground truncate font-mono text-xs'
                          title={channel.endpoint}
                        >
                          {channel.endpoint}
                        </p>
                        <div className='flex flex-wrap items-center gap-1.5 text-xs'>
                          <span className='text-muted-foreground'>
                            {t('Groups')}
                          </span>
                          <Button
                            type='button'
                            size='icon'
                            variant='ghost'
                            className='size-8'
                            aria-label={t('Groups')}
                            aria-expanded={assignment !== null}
                            title={t('Groups')}
                            disabled={
                              writeDisabled ||
                              configuration.routing.groups.length === 0
                            }
                            onClick={() =>
                              setGroupEditor(
                                assignment
                                  ? null
                                  : {
                                      id: channel.id,
                                      version: configuration.version,
                                      groups: [...(markedGroups ?? groups)],
                                    }
                              )
                            }
                          >
                            <Pencil className='size-3.5' />
                          </Button>
                          {(markedGroups ?? groups).length === 0 ? (
                            <span>{t('None')}</span>
                          ) : (
                            (markedGroups ?? groups).map((group) => (
                              <span
                                key={group}
                                className='bg-muted rounded px-1.5 py-0.5 [overflow-wrap:anywhere]'
                              >
                                {group}
                              </span>
                            ))
                          )}
                        </div>
                        {configuration.bindings.tags?.[channel.id] && (
                          <p
                            className='text-muted-foreground truncate text-xs'
                            title={configuration.bindings.tags[channel.id]}
                          >
                            {configuration.bindings.tags[channel.id]}
                          </p>
                        )}
                      </div>
                    </div>
                    <span
                      className={
                        channel.enabled
                          ? 'text-success text-xs font-medium'
                          : 'text-muted-foreground text-xs'
                      }
                    >
                      {channel.enabled ? t('Enabled') : t('Disabled')}
                    </span>
                    <div className='min-w-0 space-y-1 text-xs'>
                      <p className='font-medium tabular-nums'>
                        {t('{{count}} models', {
                          count: new Set(routes.map((route) => route.model))
                            .size,
                        })}
                      </p>
                      <p className='text-muted-foreground flex items-center gap-2 break-words'>
                        <NativeProviderLogo icon={provider?.icon} />
                        {provider && provider.id !== 'custom'
                          ? provider.label
                          : nativeProtocolLabel(channel.protocol, t)}
                      </p>
                    </div>
                    <dl className='native-channel-metrics text-muted-foreground min-w-0 text-xs'>
                      {routingMetrics.map(({ label, value }) => (
                        <div
                          key={label}
                          className='flex min-w-0 items-baseline gap-1.5'
                        >
                          <dt>{label}</dt>
                          <dd
                            className='text-foreground truncate font-medium tabular-nums'
                            title={value || undefined}
                          >
                            {value || '—'}
                          </dd>
                        </div>
                      ))}
                    </dl>
                    <div className='native-channel-actions'>
                      <Button
                        type='button'
                        variant='outline'
                        className='min-h-11 px-2'
                        disabled={writeDisabled || targets.length === 0}
                        onClick={() =>
                          setRoutingChannelId(
                            routingChannelId === channel.id ? null : channel.id
                          )
                        }
                      >
                        {t('Edit routing')}
                      </Button>
                      <Button
                        type='button'
                        variant='ghost'
                        className='text-destructive min-h-11 px-2'
                        disabled={writeDisabled}
                        onClick={() =>
                          setChannelToDelete({
                            id: channel.id,
                            name,
                            version: configuration.version,
                            routes: routes.map(
                              (route) => `${route.group} / ${route.model}`
                            ),
                          })
                        }
                      >
                        {t('Delete Channel')}
                      </Button>
                      <Button
                        type='button'
                        variant='outline'
                        className='min-h-11 px-2'
                        disabled={writeDisabled}
                        onClick={() => setOnboardingId(channel.id)}
                      >
                        {t('Model onboarding')}
                      </Button>
                      <Button
                        type='button'
                        variant='ghost'
                        className='min-h-11 px-2'
                        title={t('Fetch Models')}
                        disabled={writeDisabled || !accounting.observations}
                        onClick={(event) =>
                          channelEditor(channel, event.currentTarget, true)
                        }
                      >
                        <RefreshCw className='size-4' />
                        <span>{t('Fetch Models')}</span>
                      </Button>
                      <Button
                        type='button'
                        variant='secondary'
                        className='min-h-11 px-2'
                        title={t('Edit')}
                        disabled={busy}
                        onClick={(event) =>
                          channelEditor(channel, event.currentTarget)
                        }
                      >
                        <Pencil className='size-4' />
                        <span className='native-channel-action-label'>
                          {t('Edit')}
                        </span>
                      </Button>
                      {configuration.policy_sections?.includes(
                        'channel_overrides'
                      ) && (
                        <Button
                          variant='ghost'
                          size='sm'
                          className='min-h-11 px-2'
                          disabled={writeDisabled}
                          onClick={() => setOverrideId(channel.id)}
                        >
                          {t('Override Rules')}
                        </Button>
                      )}
                      <Button
                        type='button'
                        variant='ghost'
                        className='min-h-11 px-2'
                        title={t('channelSorting.catalogNote')}
                        disabled={writeDisabled}
                        onClick={() =>
                          void run(() =>
                            nativeRequest(
                              'GET',
                              `/api/channel/test/${channel.id}`
                            )
                          )
                        }
                      >
                        <Play className='size-4' />
                        <span className='native-channel-action-label'>
                          {t('Test Connection')}
                        </span>
                      </Button>
                      <Button
                        type='button'
                        variant='ghost'
                        className='min-h-11 px-2'
                        disabled={
                          writeDisabled ||
                          !secretsAllowed ||
                          !configuration.secret_storage_enabled
                        }
                        title={
                          !secretsAllowed ||
                          !configuration.secret_storage_enabled
                            ? t('Capability unavailable')
                            : t('API Keys')
                        }
                        onClick={() => setKeyChannel(channel.id)}
                      >
                        <KeyRound className='size-4' />
                        <span className='native-channel-action-label'>
                          {t('API Keys')}
                        </span>
                      </Button>
                      <Button
                        type='button'
                        variant='ghost'
                        className='min-h-11 px-2'
                        disabled={writeDisabled}
                        onClick={() =>
                          void run(() =>
                            setNativeChannelEnabled(
                              channel.id,
                              !channel.enabled,
                              configuration.version,
                              scope
                            )
                          )
                        }
                      >
                        {channel.enabled ? t('Disable') : t('Enable')}
                      </Button>
                    </div>
                  </div>
                  {assignment && (
                    <form
                      className='space-y-3 rounded-lg border p-3'
                      aria-label={t('Groups')}
                      onSubmit={async (event) => {
                        event.preventDefault()
                        if (
                          assignment.groups.length === 0 ||
                          assignment.version.routing !==
                            configuration.version.routing
                        )
                          return
                        if (
                          await run(() =>
                            updateNativeChannels(
                              scope,
                              assignment.version,
                              [channel.id],
                              { groups: assignment.groups }
                            )
                          )
                        ) {
                          setGroupEditor(null)
                        }
                      }}
                    >
                      <fieldset
                        className='flex flex-wrap gap-3'
                        disabled={writeDisabled}
                      >
                        <legend className='mb-2 text-sm font-medium'>
                          {t('Groups *')}
                        </legend>
                        {configuration.routing.groups.map(({ name: group }) => (
                          <div
                            key={group}
                            className='flex items-center gap-2 text-sm'
                          >
                            <Checkbox
                              aria-label={group}
                              checked={assignment.groups.includes(group)}
                              onCheckedChange={(checked) =>
                                setGroupEditor({
                                  ...assignment,
                                  groups: checked
                                    ? [...assignment.groups, group]
                                    : assignment.groups.filter(
                                        (value) => value !== group
                                      ),
                                })
                              }
                            />
                            <span>{group}</span>
                          </div>
                        ))}
                      </fieldset>
                      <div className='flex gap-2'>
                        <Button
                          type='submit'
                          disabled={
                            writeDisabled ||
                            assignment.groups.length === 0 ||
                            assignment.version.routing !==
                              configuration.version.routing
                          }
                        >
                          {t('Save')}
                        </Button>
                        <Button
                          type='button'
                          variant='ghost'
                          disabled={busy}
                          onClick={() => setGroupEditor(null)}
                        >
                          {t('Cancel')}
                        </Button>
                      </div>
                    </form>
                  )}
                  {routingChannelId === channel.id && (
                    <NativeChannelBatchEditor
                      key={`${configuration.version.routing}:${channel.id}`}
                      selectedIds={[channel.id]}
                      configuration={configuration}
                      enforceMinimum={channel.enabled}
                      busy={busy}
                      writable={configurationWritable}
                      onSave={async (patch) => {
                        if (
                          await run(() =>
                            updateNativeChannels(
                              scope,
                              configuration.version,
                              [channel.id],
                              patch
                            )
                          )
                        ) {
                          setRoutingChannelId(null)
                        }
                      }}
                      onCancel={() => setRoutingChannelId(null)}
                    />
                  )}
                  {serverChannels && (
                    <div>
                      <NativeChannelStatisticsSummary
                        value={statisticsById.get(channel.id)}
                      />
                    </div>
                  )}
                  <details className='group min-w-0'>
                    <summary className='text-muted-foreground hover:text-foreground w-fit cursor-pointer py-1.5 text-xs font-medium focus-visible:outline-2 focus-visible:outline-offset-4'>
                      {t('Details')}
                    </summary>
                    <div className='border-border/60 mt-2 min-w-0 space-y-4 border-t pt-3'>
                      {configuration.bindings.tags?.[channel.id] && (
                        <dl className='space-y-1 text-xs'>
                          <dt className='text-muted-foreground'>{t('Tag')}</dt>
                          <dd className='break-words'>
                            {configuration.bindings.tags[channel.id]}
                          </dd>
                        </dl>
                      )}
                      {serverChannels && (
                        <NativeChannelStatisticsRow
                          value={statisticsById.get(channel.id)}
                        />
                      )}
                      {routes.length > 0 && (
                        <div className='overflow-x-auto'>
                          <table
                            className='w-full text-left text-xs'
                            aria-label={t('Models')}
                          >
                            <thead className='text-muted-foreground'>
                              <tr>
                                <th className='px-2 py-2 font-medium'>
                                  {t('Models')}
                                </th>
                                <th className='px-2 py-2 font-medium'>
                                  {t('Group')}
                                </th>
                                <th className='px-2 py-2 font-medium'>
                                  {t('Priority')}
                                </th>
                                <th className='px-2 py-2 font-medium'>
                                  {t('Weight')}
                                </th>
                              </tr>
                            </thead>
                            <tbody>
                              {routes.map((route) => {
                                const targets = route.targets.filter(
                                  (target) => target.upstream_id === channel.id
                                )
                                return (
                                  <tr key={route.id}>
                                    <td className='px-2 py-2 font-mono'>
                                      {route.model}
                                    </td>
                                    <td className='px-2 py-2'>{route.group}</td>
                                    <td className='px-2 py-2 tabular-nums'>
                                      {targets
                                        .map((target) => target.priority)
                                        .join(', ')}
                                    </td>
                                    <td className='px-2 py-2 tabular-nums'>
                                      {targets
                                        .map((target) => target.weight)
                                        .join(', ')}
                                    </td>
                                  </tr>
                                )
                              })}
                            </tbody>
                          </table>
                        </div>
                      )}
                      {discovery && (
                        <details>
                          <summary className='cursor-pointer text-sm'>
                            {t('Models')} · {discovery.models.length}
                          </summary>
                          <time
                            className='text-muted-foreground text-xs'
                            dateTime={new Date(
                              discovery.observed_at * 1000
                            ).toISOString()}
                          >
                            {new Date(
                              discovery.observed_at * 1000
                            ).toLocaleString(
                              toIntlLocale(
                                i18n.resolvedLanguage ?? i18n.language
                              )
                            )}
                          </time>
                          <pre className='mt-2 max-h-48 overflow-auto text-xs whitespace-pre-wrap'>
                            {discovery.models.join('\n')}
                          </pre>
                        </details>
                      )}
                      <div className='flex flex-wrap gap-2'>
                        {accounting.usage && (
                          <Button
                            type='button'
                            variant='outline'
                            className='min-h-11'
                            onClick={() => setUsageChannel(channel)}
                          >
                            {t('channelUsage.open')}
                          </Button>
                        )}
                        {accounting.counters && (
                          <Button
                            type='button'
                            variant='outline'
                            className='min-h-11'
                            onClick={() => setBalanceChannel(channel)}
                          >
                            {t('channelBalance.title')}
                          </Button>
                        )}
                        {accounting.balanceProbe && (
                          <Button
                            type='button'
                            variant='outline'
                            className='min-h-11'
                            disabled={writeDisabled}
                            onClick={() => setBalanceConfigChannel(channel)}
                          >
                            {t('channelBalance.configure')}
                          </Button>
                        )}
                        <Button
                          type='button'
                          variant='outline'
                          className='min-h-11'
                          disabled={
                            writeDisabled ||
                            !secretsAllowed ||
                            !configuration.secret_storage_enabled
                          }
                          title={
                            !secretsAllowed ||
                            !configuration.secret_storage_enabled
                              ? t('Capability unavailable')
                              : undefined
                          }
                          onClick={() => {
                            setCopyError(null)
                            setCopyChannel(channel)
                          }}
                        >
                          {t('Copy Channel')}
                        </Button>
                      </div>
                    </div>
                  </details>
                </article>
              )
            })}
          </div>
        </>
      )}
      {channelToDelete && (
        <ConfirmDialog
          open
          onOpenChange={(open) => {
            if (!open && !busy) setChannelToDelete(null)
          }}
          title={t('Delete Channel')}
          desc={t(
            'Are you sure you want to delete channel "{{name}}"? This action cannot be undone.',
            { name: channelToDelete.name }
          )}
          destructive
          disabled={
            writeDisabled ||
            channelToDelete.version.routing !== configuration.version.routing
          }
          isLoading={busy}
          confirmText={t('Confirm delete')}
          handleConfirm={async () => {
            const deleted = await run(() =>
              nativeRequest(
                'DELETE',
                `${NATIVE_ROOT}/channels/${channelToDelete.id}`,
                {
                  expected_version: channelToDelete.version,
                }
              )
            )
            if (deleted) setChannelToDelete(null)
          }}
        >
          {channelToDelete.routes.length > 0 && (
            <div className='max-h-48 space-y-1 overflow-auto text-sm'>
              <p className='font-medium'>{t('Routes')}</p>
              <ul>
                {channelToDelete.routes.map((route, index) => (
                  <li key={index}>{route}</li>
                ))}
              </ul>
            </div>
          )}
        </ConfirmDialog>
      )}
      {section === 'models' && (
        <>
          <Button
            type='button'
            className='min-h-11'
            disabled={writeDisabled}
            onClick={() => modelEditor()}
          >
            {t('Add Model')}
          </Button>
          <div className='divide-border divide-y'>
            {models.slice(start, start + 25).map((route) => (
              <div
                key={route.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-4 py-5'
              >
                <div className='min-w-0'>
                  <h3 className='flex items-center gap-2 font-medium [overflow-wrap:anywhere]'>
                    <NativeProviderLogo
                      icon={nativeModelLogo(
                        configuration,
                        configuration.bindings.vendors[
                          configuration.bindings.models[route.id]?.vendor_id ??
                            0
                        ]?.name,
                        route.model
                      )}
                      model
                    />
                    {configuration.bindings.models[route.id]?.display_name ??
                      route.model}
                  </h3>
                  <p className='text-muted-foreground text-sm [overflow-wrap:anywhere]'>
                    {route.model} · {route.group} ·{' '}
                    {configuration.bindings.models[route.id]?.tags?.join(' · ')}{' '}
                    {route.enabled ? t('Enabled') : t('Disabled')}
                  </p>
                </div>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={writeDisabled}
                    onClick={() => modelEditor(route)}
                  >
                    {t('Edit')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/models/${route.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      {section === 'vendors' && (
        <>
          <Button
            type='button'
            className='min-h-11'
            disabled={writeDisabled}
            onClick={() => vendorEditor()}
          >
            {t('Create')}
          </Button>
          <div className='divide-border divide-y'>
            {vendors.slice(start, start + 25).map((vendor) => (
              <div
                key={vendor.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-4 py-5'
              >
                <div className='min-w-0'>
                  <h3 className='font-medium [overflow-wrap:anywhere]'>
                    {vendor.name}
                  </h3>
                  <p className='text-muted-foreground text-sm [overflow-wrap:anywhere]'>
                    {vendor.description}
                  </p>
                </div>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={writeDisabled}
                    onClick={() => vendorEditor(vendor)}
                  >
                    {t('Edit')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/vendors/${vendor.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      {section !== 'channels' && total === 0 && (
        <div role='status'>
          <EmptyState
            className='min-h-48'
            title={
              search.trim()
                ? t('No results found')
                : section === 'unpriced'
                  ? t('No unpriced models')
                  : t('No Data')
            }
            description={
              search.trim()
                ? t('Try a different search or clear the current filter.')
                : section === 'models'
                  ? t(
                      'Add a new model to the system by providing the necessary information.'
                    )
                  : section === 'vendors'
                    ? t('Add a new vendor to the system')
                    : section === 'prices'
                      ? t(
                          'No price rules yet. Add a model price to get started.'
                        )
                      : undefined
            }
            action={
              search.trim() ? (
                <Button
                  type='button'
                  variant='outline'
                  className='min-h-11'
                  onClick={() => {
                    setSearch('')
                    setPage(1)
                  }}
                >
                  {t('Clear filters')}
                </Button>
              ) : undefined
            }
          />
        </div>
      )}
      {section === 'unpriced' && unpriced.length > 0 && (
        <ul className='divide-y'>
          {unpriced.slice(start, start + 25).map(({ channel, name, model }) => (
            <li
              key={JSON.stringify([channel.id, model])}
              className='flex flex-wrap items-center justify-between gap-3 py-3'
            >
              <div className='min-w-0'>
                <p className='font-medium [overflow-wrap:anywhere]'>{model}</p>
                <p className='text-muted-foreground text-sm [overflow-wrap:anywhere]'>
                  {name} #{channel.id}
                </p>
              </div>
              <Button
                type='button'
                variant='outline'
                disabled={writeDisabled}
                onClick={() => {
                  setOnboardingModel(model)
                  setOnboardingId(channel.id)
                }}
              >
                {t('Model onboarding')}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {section === 'prices' && (
        <>
          <div className='flex flex-wrap gap-2'>
            <Button
              type='button'
              className='min-h-11'
              disabled={writeDisabled}
              onClick={() => priceEditor()}
            >
              {t('Edit')}
            </Button>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={writeDisabled}
              onClick={() => draftEditor()}
            >
              {t('Save draft')}
            </Button>
          </div>
          <NativePriceRuleList
            prices={{
              ...configuration.prices,
              rules: prices.filter((rule) =>
                visiblePriceModels.has(String(rule.model))
              ),
            }}
          />
          <details className='min-w-0'>
            <summary className='text-muted-foreground focus-visible:ring-ring w-fit cursor-pointer rounded-sm py-2 text-sm focus-visible:ring-2 focus-visible:outline-none'>
              {t('Raw JSON')}
            </summary>
            <pre className='bg-muted/40 mt-2 max-h-80 overflow-auto rounded-lg p-4 text-xs'>
              {pretty(configuration.prices)}
            </pre>
          </details>
          <div className='divide-border divide-y'>
            {Object.values(configuration.bindings.price_drafts).map((draft) => (
              <div
                key={draft.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-4 py-5'
              >
                <span className='font-medium [overflow-wrap:anywhere]'>
                  {draft.name}
                </span>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={writeDisabled}
                    onClick={() => draftEditor(draft)}
                  >
                    {t('Edit')}
                  </Button>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={
                      writeDisabled ||
                      draft.base_pricing_version !==
                        configuration.version.pricing
                    }
                    onClick={() =>
                      openEditor({
                        title: draft.name,
                        method: 'POST',
                        path: `${NATIVE_ROOT}/prices/drafts/${draft.id}/publish`,
                        previewPath: `${NATIVE_ROOT}/prices/drafts/${draft.id}/preview`,
                        document: pretty({
                          expected_version: configuration.version,
                        }),
                        readOnly: true,
                      })
                    }
                  >
                    {t('Preview')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/prices/drafts/${draft.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      <div className='flex flex-wrap items-center justify-end gap-3'>
        {paginationKnown && (
          <span className='text-sm'>
            {t('Total')}: {total}
          </span>
        )}
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={
            shownPage === 1 ||
            busy ||
            (serverChannels && sortedChannels.isFetching)
          }
          onClick={() => setPage(shownPage - 1)}
        >
          {t('Previous')}
        </Button>
        {paginationKnown && (
          <span className='text-sm'>
            {t('Page')} {shownPage} / {lastPage}
          </span>
        )}
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={
            shownPage * 25 >= total ||
            busy ||
            (serverChannels &&
              (sortedChannels.isFetching || sortedChannels.isError))
          }
          onClick={() => setPage(shownPage + 1)}
        >
          {t('Next')}
        </Button>
      </div>
      {onboardingId !== null && (
        <NativeChannelOnboarding
          key={onboardingId}
          scope={scope}
          configuration={configuration}
          channelId={onboardingId}
          initialModel={onboardingModel}
          onRefresh={async () => {
            const result = await query.refetch({ throwOnError: true })
            if (!result.data) throw new Error(t('Failed to load data'))
          }}
          onClose={() => {
            setOnboardingId(null)
            setOnboardingModel(undefined)
          }}
        />
      )}
      {balanceChannel && (
        <NativeBalanceDialog
          key={balanceChannel.id}
          scope={scope}
          channelId={balanceChannel.id}
          probeConfigured={
            configurationWritable &&
            accounting.balanceProbe &&
            !!(
              configuration.bindings.balance_probes?.[balanceChannel.id] ||
              configuration.bindings.legacy_balance?.[balanceChannel.id]
            )
          }
          channelName={
            configuration.bindings.names[balanceChannel.id] ??
            t('Channel {{id}}', { id: balanceChannel.id })
          }
          onClose={() => setBalanceChannel(null)}
        />
      )}
      {balanceBatchChannels && (
        <NativeBalanceBatchDialog
          scope={scope}
          channels={balanceBatchChannels}
          canProbe={configurationWritable && accounting.balanceProbe}
          onClose={() => {
            setBalanceBatchChannels(null)
            void refresh()
          }}
        />
      )}
      {overrideId !== null && configurationWritable && (
        <NativeChannelOverridesDialog
          key={overrideId}
          id={overrideId}
          anthropic={configuration.routing.upstreams.some(
            (upstream) =>
              upstream.id === overrideId &&
              upstream.protocol === 'anthropic_messages'
          )}
          onClose={() => setOverrideId(null)}
          onPublished={() => {
            setOverrideId(null)
            void refresh()
          }}
        />
      )}
      {balanceConfigChannel && (
        <NativeBalanceConfigDialog
          key={balanceConfigChannel.id}
          scope={scope}
          channelId={balanceConfigChannel.id}
          channelName={
            configuration.bindings.names[balanceConfigChannel.id] ??
            t('Channel {{id}}', { id: balanceConfigChannel.id })
          }
          version={configuration.version}
          writable={configurationWritable && accounting.balanceProbe}
          initialProbe={
            configuration.bindings.balance_probes?.[balanceConfigChannel.id] ??
            null
          }
          onSaved={() => void refresh()}
          onClose={() => setBalanceConfigChannel(null)}
        />
      )}
      {usageChannel && accounting.usage && (
        <NativeChannelUsageDialog
          key={usageChannel.id}
          scope={scope}
          channelId={usageChannel.id}
          countersEnabled={accounting.counters}
          channelName={
            configuration.bindings.names[usageChannel.id] ??
            t('Channel {{id}}', { id: usageChannel.id })
          }
          onClose={() => {
            setUsageChannel(null)
            void refresh()
          }}
        />
      )}
      <Dialog
        open={copyChannel !== null}
        onOpenChange={(open) => {
          if (!open && !copyPending.current) setCopyChannel(null)
        }}
        title={t('Copy Channel')}
        description={
          <>
            {t('Create a copy of:')}{' '}
            <strong>
              {copyChannel &&
                (configuration.bindings.names[copyChannel.id] ??
                  t('Channel {{id}}', { id: copyChannel.id }))}
            </strong>
          </>
        }
        footer={
          <>
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => setCopyChannel(null)}
            >
              {t('Cancel')}
            </Button>
            <Button
              disabled={
                writeDisabled ||
                !secretsAllowed ||
                !configuration.secret_storage_enabled ||
                (!copyReset && !accounting.preserveCounters)
              }
              onClick={() => void submitCopy()}
            >
              {busy ? t('Copying...') : t('Copy Channel')}
            </Button>
          </>
        }
      >
        <label className='block space-y-2'>
          <span className='text-sm'>{t('Name Suffix')}</span>
          <Input
            value={copySuffix}
            disabled={busy}
            onChange={(event) => setCopySuffix(event.target.value)}
          />
        </label>
        <p className='text-muted-foreground mt-2 text-sm break-all'>
          {t('New name will be:')}{' '}
          {copyChannel &&
            (configuration.bindings.names[copyChannel.id] ??
              t('Channel {{id}}', { id: copyChannel.id }))}
          {copySuffix}
        </p>
        <div className='mt-4 flex items-center gap-2'>
          <Checkbox
            id='native-copy-reset'
            checked={copyReset}
            disabled={busy || (copyReset && !accounting.preserveCounters)}
            onCheckedChange={(checked) => {
              if (checked === true || accounting.preserveCounters) {
                setCopyReset(checked === true)
              }
            }}
          />
          <Label htmlFor='native-copy-reset'>{t('channelCopy.reset')}</Label>
        </div>
        <p className='text-muted-foreground mt-2 text-sm'>
          {t('channelCopy.note')}
        </p>
        {copyError && (
          <Alert variant='destructive' role='alert'>
            <AlertDescription>{copyError}</AlertDescription>
          </Alert>
        )}
      </Dialog>
      {keyChannel !== null && (
        <NativeKeyManageDialog
          key={`${scope.userId}:${scope.sessionId}:${keyChannel}`}
          open
          onOpenChange={(open) => {
            if (!open) setKeyChannel(null)
          }}
          scope={scope}
          channelId={keyChannel}
          bulkImportAllowed={bulkKeyImportAllowed}
          writable={
            configurationWritable &&
            secretsAllowed &&
            configuration.secret_storage_enabled
          }
          channelName={
            configuration.bindings.names[keyChannel] ??
            t('Channel {{id}}', { id: keyChannel })
          }
          onChanged={() => void refresh()}
        />
      )}
      {result !== null && (
        <details className='bg-card rounded-xl p-4'>
          <summary className='cursor-pointer text-sm font-medium'>
            {t('Operation successful')} · {t('View details')}
          </summary>
          <pre className='mt-3 max-h-80 overflow-auto text-xs' role='status'>
            {pretty(result)}
          </pre>
        </details>
      )}
    </section>
  )
}

export function NativeRoutingToggle() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <div className='space-y-4'>
      <Button
        type='button'
        variant='outline'
        className='min-h-11'
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {t('Configuration')}
      </Button>
      {open && <NativeRoutingManagement />}
    </div>
  )
}
