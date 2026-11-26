/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowDown, ArrowUp, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import {
  getDirectorySettings,
  saveDirectorySettings,
  sameDirectoryLinks,
} from '@/features/ai-directory/admin-api'
import {
  AI_DIRECTORY_CATEGORIES,
  DEFAULT_AI_DIRECTORY_LINKS,
  safeDirectoryUrl,
  type AIDirectoryCategory,
  type AIDirectoryLink,
} from '@/features/ai-directory/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { FormNavigationGuard } from '../components/form-navigation-guard'
import { SettingsSection } from '../components/settings-section'

const categoryLabels: Record<AIDirectoryCategory, string> = {
  chat: 'Chat assistants',
  research: 'Research',
  resources: 'Useful resources',
  developer: 'Developer tools',
  creative: 'Creative tools',
  other: 'More websites',
}

function validateLinks(links: AIDirectoryLink[]): Record<string, string> {
  const errors: Record<string, string> = {}
  for (const link of links) {
    if (!link.name.trim() || [...link.name].length > 80) {
      errors[link.id] = 'Enter a website name (up to 80 characters).'
    } else if (
      !safeDirectoryUrl(link.url) ||
      /\s/.test(link.url) ||
      new TextEncoder().encode(link.url).length > 2048
    ) {
      errors[link.id] = 'Enter a valid HTTP or HTTPS website address.'
    } else if (
      [...link.summary].length > 180 ||
      [...link.description].length > 1200
    ) {
      errors[link.id] = 'The summary or description is too long.'
    }
  }
  return errors
}

// The old settings route may still pass its option value during the panel migration.
export function AIDirectorySection(_props: { initialValue?: string } = {}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  if (!auth.session || auth.user?.role !== ROLE.SUPER_ADMIN) {
    return <ErrorState title={t('Access Forbidden')} />
  }
  return <DirectoryEditor key={auth.session.sid} />
}

function DirectoryEditor() {
  const { t } = useTranslation()
  const cache = useQueryClient()
  const settings = useQuery({
    queryKey: ['ai-directory-settings'],
    queryFn: getDirectorySettings,
    retry: false,
  })
  const [draft, setDraft] = useState<AIDirectoryLink[] | null>(null)
  const [activeID, setActiveID] = useState<string | null>(null)
  const [errors, setErrors] = useState<Record<string, string>>({})
  const initialLinks = settings.data ?? DEFAULT_AI_DIRECTORY_LINKS
  const links = draft ?? initialLinks
  const dirty = draft !== null && !sameDirectoryLinks(draft, initialLinks)
  const setLinks = (
    update: (current: AIDirectoryLink[]) => AIDirectoryLink[]
  ) => setDraft((current) => update(current ?? initialLinks))
  const saveSettings = useMutation({
    mutationFn: saveDirectorySettings,
    retry: false,
    onSuccess: (saved) => {
      cache.setQueryData(['ai-directory-settings'], saved)
      cache.setQueryData(['ai-directory'], saved)
      setDraft(null)
      setErrors({})
      toast.success(t('Saved successfully'))
    },
  })
  if (settings.isPending) {
    return <LoadingState message={t('Loading settings...')} />
  }
  if (settings.isError || settings.data === undefined) {
    return (
      <ErrorState
        title={t('Unable to load settings')}
        onRetry={() => void settings.refetch()}
      />
    )
  }

  const updateLink = (id: string, patch: Partial<AIDirectoryLink>) => {
    setLinks((current) =>
      current.map((link) => (link.id === id ? { ...link, ...patch } : link))
    )
    setErrors((current) => {
      if (!current[id]) return current
      const next = { ...current }
      delete next[id]
      return next
    })
  }

  const moveLink = (index: number, direction: -1 | 1) => {
    setLinks((current) => {
      const next = [...current]
      const target = index + direction
      if (target < 0 || target >= next.length) return current
      ;[next[index], next[target]] = [next[target], next[index]]
      return next
    })
  }

  const addLink = () => {
    if (links.length >= 60) return
    const id = crypto.randomUUID()
    setLinks((current) => [
      ...current,
      {
        id,
        name: '',
        url: 'https://',
        category: 'other',
        summary: '',
        description: '',
        enabled: true,
      },
    ])
    setActiveID(id)
  }

  const save = () => {
    const next = links.map((link) => ({
      ...link,
      name: link.name.trim(),
      url: link.url.trim(),
      summary: link.summary.trim(),
      description: link.description.trim(),
    }))
    const nextErrors = validateLinks(next)
    setErrors(nextErrors)
    if (Object.keys(nextErrors).length) {
      setActiveID(Object.keys(nextErrors)[0])
      toast.error(t('Check the highlighted website.'))
      return
    }
    if (next.length > 60) return
    saveSettings.mutate(next)
  }

  return (
    <SettingsSection title={t('AI directory')}>
      <FormNavigationGuard when={dirty} />
      <div className='flex flex-wrap justify-end gap-2'>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          disabled={!dirty || saveSettings.isPending}
          onClick={() => {
            setDraft(null)
            setErrors({})
            setActiveID(null)
            saveSettings.reset()
          }}
        >
          {t('Reset')}
        </Button>
        <Button
          type='button'
          size='sm'
          disabled={!dirty || saveSettings.isPending}
          onClick={save}
        >
          {saveSettings.isPending ? t('Saving...') : t('Save websites')}
        </Button>
      </div>
      {saveSettings.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Failed to save')}
        </p>
      )}
      <div className='flex flex-wrap items-start justify-between gap-4'>
        <p className='text-muted-foreground max-w-2xl text-sm leading-relaxed'>
          {t(
            'Add and arrange websites shown in the Ecosystem directory. Changes are visible to everyone after saving.'
          )}
        </p>
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={addLink}
          disabled={saveSettings.isPending || links.length >= 60}
        >
          <Plus data-icon='inline-start' />
          {t('Add website')}
        </Button>
      </div>
      {links.length === 0 && (
        <p className='text-muted-foreground rounded-xl border border-dashed p-8 text-center text-sm'>
          {t('No websites are configured. Add one to start the directory.')}
        </p>
      )}
      <fieldset
        disabled={saveSettings.isPending}
        className='divide-border min-w-0 overflow-hidden rounded-xl border'
      >
        {links.map((link, index) => (
          <div key={link.id} className='border-border border-b last:border-b-0'>
            <div className='flex min-w-0 flex-wrap items-center gap-3 px-4 py-3'>
              <button
                type='button'
                className='focus-visible:ring-ring min-w-0 flex-1 rounded text-left focus-visible:ring-2'
                aria-expanded={activeID === link.id}
                onClick={() =>
                  setActiveID(activeID === link.id ? null : link.id)
                }
              >
                <span className='block truncate text-sm font-semibold'>
                  {link.name || t('New website')}
                </span>
                <span className='text-muted-foreground block truncate text-xs'>
                  {link.url}
                </span>
              </button>
              {!link.enabled && (
                <span className='text-muted-foreground text-xs'>
                  {t('Hidden from directory')}
                </span>
              )}
              <Button
                type='button'
                variant='ghost'
                size='icon-sm'
                disabled={index === 0}
                onClick={() => moveLink(index, -1)}
                aria-label={t('Move {{name}} up', {
                  name: link.name || t('New website'),
                })}
              >
                <ArrowUp />
              </Button>
              <Button
                type='button'
                variant='ghost'
                size='icon-sm'
                disabled={index === links.length - 1}
                onClick={() => moveLink(index, 1)}
                aria-label={t('Move {{name}} down', {
                  name: link.name || t('New website'),
                })}
              >
                <ArrowDown />
              </Button>
              <Button
                type='button'
                variant='ghost'
                size='icon-sm'
                onClick={() =>
                  setLinks((current) =>
                    current.filter((item) => item.id !== link.id)
                  )
                }
                aria-label={t('Remove {{name}}', {
                  name: link.name || t('New website'),
                })}
              >
                <Trash2 />
              </Button>
            </div>
            {activeID === link.id && (
              <div className='bg-muted/25 grid gap-4 border-t p-4 lg:grid-cols-2'>
                <div className='grid gap-1.5'>
                  <Label htmlFor={`directory-name-${link.id}`}>
                    {t('Website name')}
                  </Label>
                  <Input
                    id={`directory-name-${link.id}`}
                    value={link.name}
                    onChange={(event) =>
                      updateLink(link.id, { name: event.target.value })
                    }
                  />
                </div>
                <div className='grid gap-1.5'>
                  <Label htmlFor={`directory-url-${link.id}`}>
                    {t('Website address')}
                  </Label>
                  <Input
                    id={`directory-url-${link.id}`}
                    type='url'
                    value={link.url}
                    onChange={(event) =>
                      updateLink(link.id, { url: event.target.value })
                    }
                    placeholder='https://example.com'
                  />
                </div>
                <div className='grid gap-1.5'>
                  <Label htmlFor={`directory-category-${link.id}`}>
                    {t('Category')}
                  </Label>
                  <NativeSelect
                    className='w-full'
                    id={`directory-category-${link.id}`}
                    value={link.category}
                    onChange={(event) =>
                      updateLink(link.id, {
                        category: event.target.value as AIDirectoryCategory,
                      })
                    }
                  >
                    {AI_DIRECTORY_CATEGORIES.map((value) => (
                      <NativeSelectOption key={value} value={value}>
                        {t(categoryLabels[value])}
                      </NativeSelectOption>
                    ))}
                  </NativeSelect>
                </div>
                <div className='flex items-center justify-between gap-4 rounded-xl border px-4 py-3'>
                  <div>
                    <Label htmlFor={`directory-visible-${link.id}`}>
                      {t('Show in directory')}
                    </Label>
                    <p className='text-muted-foreground mt-1 text-xs'>
                      {t('Hidden websites stay in your configuration.')}
                    </p>
                  </div>
                  <Switch
                    id={`directory-visible-${link.id}`}
                    checked={link.enabled}
                    onCheckedChange={(enabled) =>
                      updateLink(link.id, { enabled })
                    }
                  />
                </div>
                <div className='grid gap-1.5 lg:col-span-2'>
                  <Label htmlFor={`directory-summary-${link.id}`}>
                    {t('Short summary')}
                  </Label>
                  <Input
                    id={`directory-summary-${link.id}`}
                    value={link.summary}
                    onChange={(event) =>
                      updateLink(link.id, { summary: event.target.value })
                    }
                  />
                </div>
                <div className='grid gap-1.5 lg:col-span-2'>
                  <Label htmlFor={`directory-description-${link.id}`}>
                    {t('Expanded description')}
                  </Label>
                  <Textarea
                    id={`directory-description-${link.id}`}
                    rows={3}
                    value={link.description}
                    onChange={(event) =>
                      updateLink(link.id, { description: event.target.value })
                    }
                  />
                </div>
                {errors[link.id] && (
                  <p
                    className='text-destructive text-sm lg:col-span-2'
                    role='alert'
                  >
                    {t(errors[link.id])}
                  </p>
                )}
              </div>
            )}
          </div>
        ))}
      </fieldset>
    </SettingsSection>
  )
}
