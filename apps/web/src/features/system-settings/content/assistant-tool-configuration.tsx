/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { Check, PlugZap, Search, Unplug } from 'lucide-react'
import { useEffect, useId, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { marketAPI } from '@/features/tool-market/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  assistantToolRule,
  updateAssistantToolRule,
  type AssistantCatalogTool,
  type AssistantToolPolicy,
  type AssistantToolRule,
} from './assistant-tool-policy'
import { AssistantToolTextEditor } from './assistant-tool-text-editor'

export function AssistantToolConfiguration({
  tool,
  policy,
  onChange,
  onClose,
  disabled,
  children,
  rulesSupported = true,
  groupEnabled = true,
}: {
  tool: AssistantCatalogTool
  policy: AssistantToolPolicy
  onChange: (value: string) => void
  onClose: () => void
  disabled?: boolean
  children?: ReactNode
  rulesSupported?: boolean
  groupEnabled?: boolean
}) {
  const { t } = useTranslation()
  const id = useId()
  const rule = assistantToolRule(policy, tool)
  const change = (values: Partial<AssistantToolRule>) => {
    if (rulesSupported && !disabled) {
      onChange(
        updateAssistantToolRule(policy, tool.name, { ...rule, ...values })
      )
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent
        className='max-h-[90dvh] overflow-y-auto sm:max-w-2xl'
        data-testid='assistant-tool-configuration'
      >
        <DialogHeader>
          <p className='text-muted-foreground font-mono text-xs break-all'>
            {tool.name}
          </p>
          <DialogTitle>
            {t('Configure {{tool}}', { tool: t(tool.label) })}
          </DialogTitle>
          <DialogDescription>
            {t(
              'Changes stay in this draft. Use Save assistant settings to apply them.'
            )}
          </DialogDescription>
        </DialogHeader>
        <div className='space-y-6'>
          <div className='flex items-center justify-between gap-4'>
            <label htmlFor={`${id}-enabled`}>{t('Enable this tool')}</label>
            <Switch
              id={`${id}-enabled`}
              checked={policy.tools[tool.name] !== false}
              disabled={disabled}
              onCheckedChange={(enabled) =>
                onChange(
                  JSON.stringify({
                    ...policy,
                    tools: { ...policy.tools, [tool.name]: enabled },
                  })
                )
              }
            />
          </div>
          {!groupEnabled && (
            <p role='status' className='text-muted-foreground text-sm'>
              {t(
                'This tool group is disabled. Enable its group to use the saved tool settings.'
              )}
            </p>
          )}
          {!rulesSupported && (
            <p role='status' className='text-muted-foreground text-sm'>
              {t(
                'This server does not support tool-level rules yet. Tool switches and provider settings remain available.'
              )}
            </p>
          )}
          <fieldset
            disabled={disabled || !rulesSupported}
            className='space-y-6 disabled:opacity-60'
          >
            {tool.name === 'create_site_issue' && (
              <section className='space-y-3'>
                <label htmlFor={`${id}-visibility`} className='font-medium'>
                  {t('Default issue visibility')}
                </label>
                <Select
                  value={rule.default_visibility ?? 'user'}
                  onValueChange={(value) => {
                    if (value === 'user' || value === 'admin') {
                      change({ default_visibility: value })
                    }
                  }}
                >
                  <SelectTrigger
                    id={`${id}-visibility`}
                    className='w-full'
                    disabled={disabled || !rulesSupported}
                  >
                    <SelectValue>
                      {t(
                        rule.default_visibility === 'admin'
                          ? 'Administrators only'
                          : 'Reporter and administrators'
                      )}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent alignItemWithTrigger={false}>
                    <SelectItem value='user'>
                      {t('Reporter and administrators')}
                    </SelectItem>
                    <SelectItem value='admin'>
                      {t('Administrators only')}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className='text-muted-foreground text-xs'>
                  {t(
                    'User-visible reports are private to their reporter. Security reports and internal notes stay administrator-only.'
                  )}
                </p>
              </section>
            )}
            {tool.name === 'call_market_tool' && (
              <AssistantMarketConnections
                ids={rule.market_service_ids ?? []}
                disabled={disabled || !rulesSupported}
                onChange={(ids) => change({ market_service_ids: ids })}
              />
            )}
          </fieldset>
          <AssistantToolTextEditor
            key={tool.name}
            tool={tool}
            rule={rule}
            disabled={disabled || !rulesSupported}
            onChange={change}
          />
          {children}
        </div>
        <footer className='flex flex-wrap items-center justify-between gap-3 border-t pt-4'>
          <Button
            type='button'
            variant='ghost'
            disabled={disabled || !rulesSupported}
            onClick={() => onChange(updateAssistantToolRule(policy, tool.name))}
          >
            {t('Reset tool rules')}
          </Button>
          <Button type='button' onClick={onClose}>
            {t('Done')}
          </Button>
        </footer>
      </DialogContent>
    </Dialog>
  )
}

function AssistantMarketConnections({
  ids,
  disabled,
  onChange,
}: {
  ids: string[]
  disabled?: boolean
  onChange: (ids: string[]) => void
}) {
  const { t } = useTranslation()
  const userID = useAuthStore((state) => state.auth.user?.id)
  const [search, setSearch] = useState('')
  const [query, setQuery] = useState('')
  const [offset, setOffset] = useState(0)
  useEffect(() => {
    const timer = setTimeout(() => {
      setQuery(search.trim())
      setOffset(0)
    }, 250)
    return () => clearTimeout(timer)
  }, [search])
  const catalog = useQuery({
    queryKey: ['assistant-market-connectable', userID, query, offset],
    queryFn: async () => {
      const rows = await marketAPI.list(query, offset, 'remote')
      if (
        !Array.isArray(rows) ||
        rows.some(
          (row) =>
            !row ||
            typeof row.id !== 'string' ||
            typeof row.name !== 'string' ||
            row.execution_type !== 'remote'
        )
      ) {
        throw new Error('Invalid market catalogue')
      }
      return rows
    },
    retry: false,
    staleTime: 30_000,
  })
  const toggle = (id: string) =>
    onChange(
      ids.includes(id)
        ? ids.filter((current) => current !== id)
        : [...ids, id].sort()
    )
  return (
    <section className='space-y-4' aria-label={t('Tool market connections')}>
      <div>
        <h4 className='font-medium'>{t('Connect tools from the market')}</h4>
        <p className='text-muted-foreground mt-2 text-sm'>
          {t(
            'Connect a service once to make its tools discoverable. Users must still authorize each tool, version and spending limit. No service is connected by default.'
          )}
        </p>
      </div>
      <div className='relative'>
        <Search
          className='text-muted-foreground absolute start-3 top-3 size-4'
          aria-hidden='true'
        />
        <Input
          value={search}
          disabled={disabled}
          className='ps-9'
          aria-label={t('Search market services')}
          placeholder={t('Search market services')}
          onChange={(event) => setSearch(event.target.value)}
        />
      </div>
      <p className='text-muted-foreground text-xs'>
        {t('Connected {{count}} of 64 services', { count: ids.length })}
      </p>
      {ids.length > 0 && (
        <div className='flex flex-wrap gap-2'>
          {ids.map((id) => (
            <Button
              type='button'
              key={id}
              variant='secondary'
              size='sm'
              disabled={disabled}
              className='h-auto max-w-full gap-2 whitespace-normal'
              aria-label={t('Disconnect {{service}}', { service: id })}
              onClick={() => toggle(id)}
            >
              <span className='break-all'>
                {catalog.data?.find((row) => row.id === id)?.name ?? id}
              </span>
              <Unplug className='size-3 shrink-0' />
            </Button>
          ))}
        </div>
      )}
      {catalog.isPending ? (
        <p role='status'>{t('Loading...')}</p>
      ) : catalog.isError ? (
        <div role='alert'>
          <p>{t('Unable to load market services.')}</p>
          <Button
            type='button'
            variant='link'
            onClick={() => void catalog.refetch()}
          >
            {t('Retry')}
          </Button>
        </div>
      ) : (
        <div className='max-h-80 space-y-1 overflow-y-auto'>
          {catalog.data.length === 0 && (
            <p className='text-muted-foreground py-6 text-center'>
              {t('No matching remote services.')}
            </p>
          )}
          {catalog.data.map((row) => (
            <div
              key={row.id}
              className='bg-muted/30 flex items-start justify-between gap-4 rounded-xl p-3'
            >
              <div className='min-w-0'>
                <p className='font-medium break-words'>{row.name}</p>
                <p className='text-muted-foreground mt-1 line-clamp-2 text-xs break-words'>
                  {row.description}
                </p>
              </div>
              <Button
                type='button'
                size='sm'
                variant={ids.includes(row.id) ? 'secondary' : 'outline'}
                disabled={
                  disabled || (!ids.includes(row.id) && ids.length >= 64)
                }
                aria-pressed={ids.includes(row.id)}
                onClick={() => toggle(row.id)}
              >
                {ids.includes(row.id) ? (
                  <Check className='size-3.5' />
                ) : (
                  <PlugZap className='size-3.5' />
                )}
                {t(ids.includes(row.id) ? 'Connected' : 'Connect')}
              </Button>
            </div>
          ))}
        </div>
      )}
      <div className='flex justify-between gap-3'>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          disabled={offset === 0 || catalog.isFetching}
          onClick={() => setOffset(Math.max(0, offset - 30))}
        >
          {t('Previous')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          disabled={
            catalog.isFetching || !catalog.data || catalog.data.length < 30
          }
          onClick={() => setOffset(offset + 30)}
        >
          {t('Next')}
        </Button>
      </div>
    </section>
  )
}
