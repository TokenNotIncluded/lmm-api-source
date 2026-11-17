/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Switch } from '@/components/ui/switch'

import {
  updateAssistantToolPolicy,
  type AssistantCatalogTool,
  type AssistantToolPolicy,
} from './assistant-tool-policy'

export function AssistantToolConfiguration({
  tool,
  policy,
  onChange,
  onClose,
  disabled,
  children,
  groupEnabled = true,
}: {
  tool: AssistantCatalogTool
  policy: AssistantToolPolicy
  onChange: (value: string) => void
  onClose: () => void
  disabled?: boolean
  children?: ReactNode
  groupEnabled?: boolean
}) {
  const { t } = useTranslation()
  const id = useId()
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
                  updateAssistantToolPolicy(policy, 'tools', tool.name, enabled)
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
          {children}
        </div>
        <footer className='flex justify-end border-t pt-4'>
          <Button type='button' onClick={onClose}>
            {t('Done')}
          </Button>
        </footer>
      </DialogContent>
    </Dialog>
  )
}
