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
import {
  AlertCircle,
  CheckCircle2,
  Download,
  FileUp,
  RefreshCw,
  Sparkles,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/confirm-dialog'
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { requestAssistantOpen } from '@/features/assistant/assistant-events'
import { SystemJsonCodeEditor } from '@/features/system-settings/components/system-json-code-editor'
import type { SystemJsonConfigurationKey } from '@/features/system-settings/components/system-json-configurations'
import { useAuthStore } from '@/stores/auth-store'

import { updateSystemOptions, validateSystemOptions } from '../api'
import { FormNavigationGuard } from '../components/form-navigation-guard'
import { SettingsPageActionsPortal } from '../components/settings-page-context'
import { SettingsSection } from '../components/settings-section'
import { useSystemOptions } from '../hooks/use-system-options'
import {
  hasUnsavedJsonDraft,
  shouldApplyRawJsonServerValue,
} from './raw-json-draft-state'

type RawJsonDescriptor = {
  key: SystemJsonConfigurationKey
  label: string
}

// This is an explicit allowlist: the raw editor must never expose secrets or
// arbitrary option keys. Examples and field contracts come from the same
// reviewed registry used by inline settings editors.
export const RAW_JSON_DESCRIPTORS = [
  { key: 'group_ratio_setting.group_warnings', label: 'Group warnings' },
  { key: 'GroupRatio', label: 'Group ratios' },
  { key: 'GroupGroupRatio', label: 'Inter-group ratios' },
  { key: 'TopupGroupRatio', label: 'Top-up group ratios' },
  { key: 'UserUsableGroups', label: 'Selectable groups' },
  { key: 'AutoGroups', label: 'Auto group order' },
  {
    key: 'group_ratio_setting.group_special_usable_group',
    label: 'Special usable groups',
  },
  { key: 'AssistantSkillFiles', label: 'AI skill files' },
  {
    key: 'global.thinking_model_blacklist',
    label: 'Thinking model blacklist',
  },
  {
    key: 'global.chat_completions_to_responses_policy',
    label: 'Chat to Responses policy',
  },
  { key: 'gemini.safety_settings', label: 'Gemini safety settings' },
  { key: 'gemini.version_settings', label: 'Gemini version settings' },
  {
    key: 'gemini.supported_imagine_models',
    label: 'Gemini Imagine models',
  },
  { key: 'claude.model_headers_settings', label: 'Claude model headers' },
  { key: 'claude.default_max_tokens', label: 'Claude default max tokens' },
  { key: 'billing_setting.billing_mode', label: 'Billing modes' },
  { key: 'billing_setting.billing_expr', label: 'Billing expressions' },
  { key: 'tool_price_setting.prices', label: 'Tool prices' },
  { key: 'channel_affinity_setting.rules', label: 'Channel affinity rules' },
  { key: 'console_setting.api_info', label: 'API information' },
  { key: 'console_setting.announcements', label: 'Announcements' },
  { key: 'console_setting.faq', label: 'FAQ' },
  {
    key: 'payment_setting.amount_options',
    label: 'Top-up amount options',
  },
  { key: 'payment_setting.amount_discount', label: 'Top-up discounts' },
  { key: 'PayMethods', label: 'Payment methods' },
  { key: 'CreemProducts', label: 'Creem products' },
  { key: 'WaffoPayMethods', label: 'Waffo payment methods' },
] as const satisfies readonly RawJsonDescriptor[]

type RawJsonConfigurationKey = (typeof RAW_JSON_DESCRIPTORS)[number]['key']

const descriptorMap: ReadonlyMap<
  string,
  (typeof RAW_JSON_DESCRIPTORS)[number]
> = new Map(
  RAW_JSON_DESCRIPTORS.map((descriptor) => [descriptor.key, descriptor])
)

function isRawJsonConfigurationKey(
  value: string
): value is RawJsonConfigurationKey {
  return descriptorMap.has(value)
}

function formatJson(value: string) {
  if (!value.trim()) return ''
  try {
    return JSON.stringify(JSON.parse(value), null, 2)
  } catch {
    return value
  }
}

function parseImport(value: string): string {
  return JSON.stringify(JSON.parse(value), null, 2)
}

function downloadJson(filename: string, value: string) {
  const blob = new Blob([value], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  URL.revokeObjectURL(url)
}

export function RawJsonConfigurationSection() {
  const auth = useAuthStore((state) => state.auth)
  return (
    <RawJsonConfigurationEditor
      key={`${auth.user?.id}:${auth.user?.role}:${auth.session?.sid}`}
    />
  )
}

function RawJsonConfigurationEditor() {
  const { t } = useTranslation()
  const optionsQuery = useSystemOptions()
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [selectedKey, setSelectedKey] = useState<RawJsonConfigurationKey>(
    'group_ratio_setting.group_warnings'
  )
  const [editor, setEditor] = useState<{
    key: RawJsonConfigurationKey | null
    sourceValue: string
    editorValue: string
    baselineValue: string
    validationFeedback: { kind: 'success' | 'error'; message: string } | null
    pendingReplacement:
      | { kind: 'configuration'; key: RawJsonConfigurationKey }
      | { kind: 'import'; value: string }
      | null
  }>({
    key: null,
    sourceValue: '',
    editorValue: '',
    baselineValue: '',
    validationFeedback: null,
    pendingReplacement: null,
  })
  const { editorValue, baselineValue, validationFeedback, pendingReplacement } =
    editor
  const setBaselineValue = (value: string) =>
    setEditor((current) => ({ ...current, baselineValue: value }))
  const setValidationFeedback = (value: typeof validationFeedback) =>
    setEditor((current) => ({ ...current, validationFeedback: value }))
  const setPendingReplacement = (value: typeof pendingReplacement) =>
    setEditor((current) => ({ ...current, pendingReplacement: value }))
  const [isValidating, setIsValidating] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const descriptor = descriptorMap.get(selectedKey)
  const dirty = hasUnsavedJsonDraft(editorValue, baselineValue)
  const isBusy = isValidating || isSaving
  const controlsDisabled =
    optionsQuery.isLoading || optionsQuery.isError || !descriptor || isBusy
  const availableDescriptors = useMemo(
    () =>
      RAW_JSON_DESCRIPTORS.filter((item) =>
        optionsQuery.data?.data?.some((option) => option.key === item.key)
      ),
    [optionsQuery.data?.data]
  )
  const selectItems = useMemo(
    () =>
      availableDescriptors.map((item) => ({
        value: item.key,
        label: `${t(item.label)} · ${item.key}`,
      })),
    [availableDescriptors, t]
  )

  // A new key starts its own draft; a server refresh replaces only a clean
  // draft. Keep the accepted source separate from an acknowledged local save.
  if (
    !dirty &&
    availableDescriptors.length > 0 &&
    !availableDescriptors.some((item) => item.key === selectedKey)
  ) {
    setSelectedKey(availableDescriptors[0].key)
  }
  const sourceValue = formatJson(
    optionsQuery.data?.data?.find((option) => option.key === selectedKey)
      ?.value ?? ''
  )
  if (
    (editor.key !== selectedKey || editor.sourceValue !== sourceValue) &&
    shouldApplyRawJsonServerValue({
      loadedKey: editor.key,
      selectedKey,
      editorValue,
      baselineValue,
    })
  ) {
    setEditor({
      key: selectedKey,
      sourceValue,
      editorValue: sourceValue,
      baselineValue: sourceValue,
      validationFeedback: null,
      pendingReplacement: null,
    })
  }

  const requestScope = useMemo(
    () => ({ key: selectedKey, pending: new Set<AbortController>() }),
    [selectedKey]
  )
  useEffect(
    () => () => {
      for (const request of requestScope.pending) request.abort()
      requestScope.pending.clear()
    },
    [requestScope]
  )

  const replaceEditorValue = (value: string) => {
    setEditor((current) => ({
      ...current,
      editorValue: value,
      validationFeedback: null,
    }))
  }

  const requestConfigurationChange = (value: string | null) => {
    if (!value || !isRawJsonConfigurationKey(value) || value === selectedKey) {
      return
    }
    if (dirty) {
      setPendingReplacement({ kind: 'configuration', key: value })
      return
    }
    setSelectedKey(value)
  }

  const confirmPendingReplacement = () => {
    if (!pendingReplacement) return

    if (pendingReplacement.kind === 'configuration') {
      setSelectedKey(pendingReplacement.key)
    } else {
      replaceEditorValue(pendingReplacement.value)
    }
    setPendingReplacement(null)
  }

  const validate = async (operation?: AbortController) => {
    const request = operation ?? new AbortController()
    if (request.signal.aborted) return false
    if (!editorValue.trim()) {
      setValidationFeedback({
        kind: 'error',
        message: t('A JSON value is required.'),
      })
      return false
    }
    try {
      JSON.parse(editorValue)
    } catch {
      setValidationFeedback({
        kind: 'error',
        message: t('Invalid JSON. Please fix the syntax first.'),
      })
      return false
    }
    requestScope.pending.add(request)
    setIsValidating(true)
    try {
      const response = await validateSystemOptions({
        [requestScope.key]: editorValue,
      })
      if (request.signal.aborted) return false
      if (!response.success) {
        setValidationFeedback({
          kind: 'error',
          message: response.message || t('Configuration is invalid.'),
        })
        return false
      }
      setValidationFeedback({
        kind: 'success',
        message: t('Configuration validated.'),
      })
      return true
    } catch (error) {
      if (request.signal.aborted) return false
      setValidationFeedback({
        kind: 'error',
        message:
          error instanceof Error
            ? error.message
            : t('Configuration check failed.'),
      })
      return false
    } finally {
      if (!request.signal.aborted) setIsValidating(false)
      if (!operation) requestScope.pending.delete(request)
    }
  }

  const save = async () => {
    const request = new AbortController()
    try {
      if (!(await validate(request)) || request.signal.aborted) return
      setIsSaving(true)
      const response = await updateSystemOptions({
        [requestScope.key]: editorValue,
      })
      if (request.signal.aborted) return
      if (!response.success) {
        throw new Error(response.message || t('Failed to update setting'))
      }
      setBaselineValue(editorValue)
      toast.success(t('Setting updated successfully'))
      await optionsQuery.refetch()
    } catch (error) {
      if (request.signal.aborted) return
      toast.error(
        error instanceof Error ? error.message : t('Failed to update setting')
      )
    } finally {
      if (!request.signal.aborted) setIsSaving(false)
      requestScope.pending.delete(request)
    }
  }

  const handleImport = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    const request = new AbortController()
    requestScope.pending.add(request)

    void file
      .text()
      .then((text) => {
        try {
          const importedValue = parseImport(text)
          if (request.signal.aborted) return
          setEditor((current) =>
            hasUnsavedJsonDraft(current.editorValue, current.baselineValue)
              ? {
                  ...current,
                  pendingReplacement: { kind: 'import', value: importedValue },
                }
              : {
                  ...current,
                  editorValue: importedValue,
                  validationFeedback: null,
                }
          )
        } catch {
          if (request.signal.aborted) return
          setValidationFeedback({
            kind: 'error',
            message: t('The imported file is not valid JSON.'),
          })
        }
      })
      .catch((error: unknown) => {
        if (request.signal.aborted) return
        setValidationFeedback({
          kind: 'error',
          message:
            error instanceof Error
              ? error.message
              : t('Configuration check failed.'),
        })
      })
      .finally(() => requestScope.pending.delete(request))
  }

  const askAssistant = () => {
    const safeValue = editorValue.slice(0, 12_000)
    requestAssistantOpen(
      'service',
      `请帮我编辑管理员配置 ${selectedKey}。先读取当前服务端配置并解释字段，再提出最小变更预览；未经我确认不要写入。当前编辑器内容如下：\n\n${safeValue}`
    )
  }

  return (
    <SettingsSection title={t('Raw JSON Configuration')}>
      <FormNavigationGuard when={dirty} />
      <ConfirmDialog
        open={Boolean(pendingReplacement)}
        onOpenChange={(open) => {
          if (!open) setPendingReplacement(null)
        }}
        title={t('Discard unsaved JSON changes?')}
        desc={t(
          'Continuing will replace the unsaved JSON currently in the editor.'
        )}
        confirmText={t('Replace')}
        destructive
        handleConfirm={confirmPendingReplacement}
      />
      <div className='space-y-4'>
        <p
          id='raw-json-configuration-description'
          className='text-muted-foreground text-sm'
        >
          {t(
            'Edit one safe-listed JSON setting at a time. Imports are checked locally and by the server before any write.'
          )}
        </p>
        {optionsQuery.isError && (
          <Alert variant='destructive'>
            <AlertCircle />
            <AlertTitle>{t('Failed to load')}</AlertTitle>
            <AlertDescription>
              {optionsQuery.error instanceof Error
                ? optionsQuery.error.message
                : t('Configuration check failed.')}
            </AlertDescription>
            <AlertAction>
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => void optionsQuery.refetch()}
                disabled={optionsQuery.isFetching}
              >
                <RefreshCw
                  className={optionsQuery.isFetching ? 'animate-spin' : ''}
                  aria-hidden='true'
                />
                {t('Retry')}
              </Button>
            </AlertAction>
          </Alert>
        )}
        <div className='flex flex-wrap items-end gap-3'>
          <div className='min-w-0 flex-1 basis-full space-y-2 sm:basis-64'>
            <Label htmlFor='raw-json-configuration-key'>
              {t('Configuration key')}
            </Label>
            <Select
              items={selectItems}
              value={selectedKey}
              onValueChange={requestConfigurationChange}
              disabled={controlsDisabled || availableDescriptors.length === 0}
            >
              <SelectTrigger
                id='raw-json-configuration-key'
                aria-describedby='raw-json-configuration-description'
                className='w-full max-w-full'
              >
                <SelectValue placeholder={t('Select')} />
              </SelectTrigger>
              <SelectContent alignItemWithTrigger={false}>
                <SelectGroup>
                  {availableDescriptors.map((item) => (
                    <SelectItem key={item.key} value={item.key}>
                      {t(item.label)} · {item.key}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
          <input
            ref={fileInputRef}
            type='file'
            accept='application/json,.json'
            className='hidden'
            onChange={handleImport}
          />
          <Button
            type='button'
            variant='outline'
            onClick={() => fileInputRef.current?.click()}
            disabled={controlsDisabled}
          >
            <FileUp data-icon='inline-start' />
            {t('Import JSON')}
          </Button>
          <Button
            type='button'
            variant='outline'
            onClick={() => downloadJson(`${selectedKey}.json`, editorValue)}
            disabled={!editorValue.trim()}
          >
            <Download data-icon='inline-start' />
            {t('Export JSON')}
          </Button>
          <Button
            type='button'
            variant='outline'
            onClick={askAssistant}
            disabled={isBusy}
          >
            <Sparkles data-icon='inline-start' />
            {t('Ask AI to edit')}
          </Button>
        </div>
        <SystemJsonCodeEditor
          configurationKey={selectedKey}
          specificationDefaultOpen
          value={editorValue}
          onChange={replaceEditorValue}
          disabled={controlsDisabled}
          heightClassName='h-80 min-h-80 max-h-80 sm:h-[28rem] sm:min-h-[28rem] sm:max-h-[28rem]'
          ariaLabel={`${t('JSON')} — ${descriptor ? t(descriptor.label) : selectedKey}`}
        />
        {validationFeedback && (
          <Alert
            variant={
              validationFeedback.kind === 'success' ? 'default' : 'destructive'
            }
          >
            {validationFeedback.kind === 'success' ? (
              <CheckCircle2 />
            ) : (
              <AlertCircle />
            )}
            <AlertTitle>{t('Configuration check')}</AlertTitle>
            <AlertDescription>{validationFeedback.message}</AlertDescription>
          </Alert>
        )}
        <SettingsPageActionsPortal>
          <Button
            type='button'
            variant='outline'
            onClick={() => void validate()}
            disabled={controlsDisabled || !dirty}
          >
            {isValidating ? t('Checking...') : t('Validate configuration')}
          </Button>
          <Button
            type='button'
            onClick={() => void save()}
            disabled={controlsDisabled || !dirty}
          >
            {isSaving ? t('Saving...') : t('Save Changes')}
          </Button>
        </SettingsPageActionsPortal>
      </div>
    </SettingsSection>
  )
}
