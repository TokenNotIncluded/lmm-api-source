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
/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { ArrowDown, ArrowUp, Plus, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { INTERFACE_LANGUAGE_OPTIONS } from '@/i18n/languages'

import { SettingsDisclosure } from '../components/settings-disclosure'

// A starter keeps one administrator default for every language and may add an
// explicit translation per interface language. Stored as "default" plus locale
// keys so the server can select copy without client-side guessing.
type ConversationStarterCopy = Record<string, string>
type ConversationStarter = {
  id: string
  label: ConversationStarterCopy
  prompt: ConversationStarterCopy
}

const DEFAULT_CONVERSATION_STARTERS: ConversationStarter[] = [
  {
    id: 'getting_started',
    label: { default: 'Where should I start?' },
    prompt: { default: 'Where should I start?' },
  },
  {
    id: 'new_user_gift',
    label: { default: 'How do I get the new-user gift?' },
    prompt: { default: 'How do I get the new-user gift?' },
  },
  {
    id: 'weekly_discount',
    label: { default: 'Any top-up discounts this week?' },
    prompt: { default: 'Any top-up discounts this week?' },
  },
  {
    id: 'ai_recommendation',
    label: { default: 'How do I apply for L1 access?' },
    prompt: { default: 'How do I apply for L1 access?' },
  },
]

function normalizeConversationStarterCopy(
  value: unknown
): ConversationStarterCopy | null {
  // Legacy rows store one plain string; treat it as the default language.
  if (typeof value === 'string') {
    return { default: value }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const copy: ConversationStarterCopy = {}
  for (const [locale, text] of Object.entries(
    value as Record<string, unknown>
  )) {
    if (typeof text !== 'string') continue
    copy[locale] = text
  }
  // A freshly added starter has no copy yet; keep its row so it can be edited.
  if (!Object.hasOwn(copy, 'default')) copy.default = ''
  return copy
}

function parseConversationStarters(value: string): ConversationStarter[] {
  if (!value.trim()) return DEFAULT_CONVERSATION_STARTERS
  try {
    const parsed = JSON.parse(value) as unknown
    if (!Array.isArray(parsed)) return []
    const starters: ConversationStarter[] = []
    for (const item of parsed) {
      if (!item || typeof item !== 'object') continue
      const candidate = item as Record<string, unknown>
      if (typeof candidate.id !== 'string') continue
      const label = normalizeConversationStarterCopy(candidate.label)
      const prompt = normalizeConversationStarterCopy(candidate.prompt)
      if (!label || !prompt) continue
      starters.push({ id: candidate.id, label, prompt })
    }
    return starters
  } catch {
    return []
  }
}

export function ConversationStartersEditor(props: {
  value: string
  disabled: boolean
  onChange: (value: string) => void
}) {
  const { t } = useTranslation()
  const items = parseConversationStarters(props.value)
  const write = (next: ConversationStarter[]) =>
    props.onChange(JSON.stringify(next))
  const update = (index: number, patch: Partial<ConversationStarter>) =>
    write(
      items.map((item, itemIndex) =>
        itemIndex === index ? { ...item, ...patch } : item
      )
    )
  const updateCopy = (
    index: number,
    field: 'label' | 'prompt',
    locale: string,
    text: string
  ) =>
    update(index, {
      [field]: { ...items[index][field], [locale]: text },
    } as Partial<ConversationStarter>)
  const clearCopy = (
    index: number,
    field: 'label' | 'prompt',
    locale: string
  ) =>
    update(index, {
      [field]: Object.fromEntries(
        Object.entries(items[index][field]).filter(([key]) => key !== locale)
      ),
    } as Partial<ConversationStarter>)
  const remove = (index: number) =>
    write(items.filter((_, itemIndex) => itemIndex !== index))
  const move = (index: number, direction: -1 | 1) => {
    const target = index + direction
    if (target < 0 || target >= items.length) return
    const next = [...items]
    ;[next[index], next[target]] = [next[target], next[index]]
    write(next)
  }
  return (
    <div
      className='space-y-3'
      data-testid='assistant-conversation-starters-editor'
    >
      <div className='text-muted-foreground text-sm'>
        {t(
          'Each starter shows in the visitor’s interface language when you provide that language, and otherwise falls back to the default text.'
        )}
      </div>
      {items.map((item, index) => (
        <SettingsDisclosure
          key={item.id}
          className='settings-starter'
          defaultOpen={index === 0 || !item.label.default}
          title={
            <>
              <span className='settings-starter-index'>
                {String(index + 1).padStart(2, '0')}
              </span>
              {item.label.default || t('Button label')}
            </>
          }
        >
          <div className='space-y-3'>
            <div className='flex items-center justify-between gap-2'>
              <span className='text-muted-foreground text-xs'>
                {t('Starter {{number}}', { number: index + 1 })}
              </span>
              <div className='flex gap-1'>
                <Button
                  type='button'
                  variant='ghost'
                  size='icon'
                  aria-label={t('Move starter up')}
                  onClick={() => move(index, -1)}
                  disabled={props.disabled || index === 0}
                >
                  <ArrowUp className='size-4' />
                </Button>
                <Button
                  type='button'
                  variant='ghost'
                  size='icon'
                  aria-label={t('Move starter down')}
                  onClick={() => move(index, 1)}
                  disabled={props.disabled || index === items.length - 1}
                >
                  <ArrowDown className='size-4' />
                </Button>
                <Button
                  type='button'
                  variant='ghost'
                  size='icon'
                  aria-label={t('Delete starter')}
                  onClick={() => remove(index)}
                  disabled={props.disabled}
                >
                  <Trash2 className='size-4' />
                </Button>
              </div>
            </div>
            <Input
              aria-label={t('Button label')}
              value={item.label.default ?? ''}
              maxLength={80}
              onChange={(event) =>
                updateCopy(index, 'label', 'default', event.target.value)
              }
              disabled={props.disabled}
            />
            <Textarea
              aria-label={t('Prompt text')}
              value={item.prompt.default ?? ''}
              maxLength={2000}
              rows={2}
              onChange={(event) =>
                updateCopy(index, 'prompt', 'default', event.target.value)
              }
              disabled={props.disabled}
            />
            <SettingsDisclosure title={t('Translations')}>
              <div className='space-y-4'>
                {INTERFACE_LANGUAGE_OPTIONS.map((language) => (
                  <div
                    key={language.code}
                    className='grid gap-2 sm:grid-cols-[8rem_minmax(0,1fr)] sm:items-center'
                  >
                    <span className='text-muted-foreground text-xs'>
                      {language.label}
                    </span>
                    <Input
                      aria-label={t('{{language}} label', {
                        language: language.label,
                      })}
                      value={item.label[language.code] ?? ''}
                      maxLength={80}
                      placeholder={item.label.default ?? ''}
                      onChange={(event) =>
                        event.target.value
                          ? updateCopy(
                              index,
                              'label',
                              language.code,
                              event.target.value
                            )
                          : clearCopy(index, 'label', language.code)
                      }
                      disabled={props.disabled}
                    />
                    <span className='text-muted-foreground text-xs sm:col-start-1'>
                      {t('Prompt')}
                    </span>
                    <Textarea
                      aria-label={t('{{language}} prompt', {
                        language: language.label,
                      })}
                      value={item.prompt[language.code] ?? ''}
                      maxLength={2000}
                      rows={2}
                      placeholder={item.prompt.default ?? ''}
                      onChange={(event) =>
                        event.target.value
                          ? updateCopy(
                              index,
                              'prompt',
                              language.code,
                              event.target.value
                            )
                          : clearCopy(index, 'prompt', language.code)
                      }
                      disabled={props.disabled}
                    />
                  </div>
                ))}
              </div>
            </SettingsDisclosure>
          </div>
        </SettingsDisclosure>
      ))}
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={() =>
            write([
              ...items,
              {
                id: `custom_${crypto.randomUUID()}`,
                label: { default: '' },
                prompt: { default: '' },
              },
            ])
          }
          disabled={props.disabled || items.length >= 20}
        >
          <Plus className='mr-1 size-4' />
          {t('Add starter')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          onClick={() => props.onChange('')}
          disabled={props.disabled}
        >
          {t('Restore defaults')}
        </Button>
      </div>
    </div>
  )
}
