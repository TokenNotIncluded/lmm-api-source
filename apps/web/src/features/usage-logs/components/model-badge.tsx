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
import { Alert02Icon, Route01Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { Boxes } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { CopyButton } from '@/components/copy-button'
import { StatusBadge } from '@/components/status-badge'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { getLobeIcon } from '@/lib/lobe-icon'
import { resolveModelProvider } from '@/lib/model-provider'
import { cn } from '@/lib/utils'

import {
  isResponseModelMismatch,
  type ResponseModelObservation,
} from '../lib/response-model'

interface ModelBadgeProps {
  modelName: string
  actualModel?: string
  responseModel?: ResponseModelObservation
  className?: string
  detailsOnly?: boolean
}

function ModelBadgeContent(props: ModelBadgeProps & { copyable?: boolean }) {
  const provider = resolveModelProvider(props.modelName)

  return (
    <StatusBadge
      copyText={props.modelName}
      copyable={props.copyable}
      size='sm'
      showDot={false}
      autoColor={provider ? undefined : props.modelName}
      className={cn(
        'border-border/60 bg-muted/30 h-auto min-h-6 max-w-full gap-1.5 rounded-md border px-2 py-0.5 whitespace-normal [font-family:var(--font-body)]',
        provider && 'text-foreground',
        props.className
      )}
    >
      <span className='flex max-w-full min-w-0 items-center gap-1.5'>
        <span
          className='flex size-[18px] shrink-0 items-center justify-center'
          title={provider?.label}
          aria-hidden='true'
        >
          {provider?.icon ? (
            getLobeIcon(provider.icon, 18)
          ) : (
            <Boxes className='size-3.5' />
          )}
        </span>
        <span className='min-w-0 [overflow-wrap:anywhere]'>
          {props.modelName}
        </span>
      </span>
    </StatusBadge>
  )
}

export function ModelBadge(props: ModelBadgeProps) {
  const { t } = useTranslation()
  const mismatch = isResponseModelMismatch(props.responseModel)
  const returnedLabel = props.responseModel
    ? t('Response model: {{model}}', {
        model: props.responseModel.returned_model,
      })
    : ''
  const hasDetails =
    !!props.actualModel ||
    !!(
      props.responseModel &&
      (mismatch ||
        props.responseModel.returned_model !==
          props.responseModel.requested_model ||
        props.responseModel.upstream_model !==
          props.responseModel.requested_model)
    )

  if (!hasDetails) {
    return props.detailsOnly ? null : <ModelBadgeContent {...props} />
  }

  return (
    <Popover>
      <PopoverTrigger
        render={
          <button
            type='button'
            aria-label={`${t('Model')}: ${props.modelName}${returnedLabel ? `, ${returnedLabel}` : ''}`}
            className='inline-flex max-w-full min-w-0 flex-wrap items-center gap-1 text-left'
          />
        }
      >
        {!props.detailsOnly && (
          <ModelBadgeContent {...props} copyable={false} />
        )}
        {mismatch && props.responseModel ? (
          <ResponseModelWarning observation={props.responseModel} />
        ) : (
          <span className='text-muted-foreground shrink-0 [&>svg]:size-3'>
            <HugeiconsIcon icon={Route01Icon} aria-hidden='true' />
          </span>
        )}
        {props.detailsOnly && !mismatch && (
          <span className='text-muted-foreground min-w-0 truncate font-mono text-xs'>
            {props.actualModel ?? props.responseModel?.upstream_model}
          </span>
        )}
      </PopoverTrigger>
      <PopoverContent className='w-96 max-w-[calc(100vw-2rem)]'>
        {props.responseModel ? (
          <ResponseModelDetails observation={props.responseModel} />
        ) : (
          <div className='flex flex-col gap-2'>
            <ModelDetailRow
              label={t('Request Model:')}
              model={props.modelName}
            />
            <ModelDetailRow
              label={t('Actual Model:')}
              model={props.actualModel || ''}
            />
          </div>
        )}
      </PopoverContent>
    </Popover>
  )
}

function ModelDetailRow(props: { label: string; model: string }) {
  const { t } = useTranslation()
  const hasModelName = props.model.trim() !== ''
  return (
    <div className='grid min-w-0 grid-cols-[5.25rem_minmax(0,1fr)_auto] items-start gap-2 text-xs sm:grid-cols-[7rem_minmax(0,1fr)_auto] sm:gap-3'>
      <span className='text-muted-foreground min-w-0'>{props.label}</span>
      <span className='min-w-0 font-mono [overflow-wrap:anywhere]'>
        {hasModelName ? props.model : '—'}
      </span>
      {hasModelName && (
        <CopyButton
          value={props.model}
          aria-label={`${t('Copy to clipboard')} (${props.label})`}
          className='size-6'
        />
      )}
    </div>
  )
}

function ResponseModelWarning(props: {
  observation: ResponseModelObservation
}) {
  const { t } = useTranslation()
  return (
    <StatusBadge
      variant='warning'
      copyable={false}
      aria-label={t('Response model mismatch')}
      data-response-model-warning
      className='h-auto min-h-5 max-w-full py-0.5 text-left whitespace-normal'
    >
      <span className='shrink-0 [&>svg]:size-3.5'>
        <HugeiconsIcon icon={Alert02Icon} aria-hidden='true' />
      </span>
      <span className='min-w-0 [overflow-wrap:anywhere]'>
        {t('Response model: {{model}}', {
          model: props.observation.returned_model,
        })}
      </span>
    </StatusBadge>
  )
}

/** Shared by the table/mobile popover and the request details dialog. */
export function ResponseModelDetails(props: {
  observation: ResponseModelObservation
}) {
  const { t } = useTranslation()
  const mismatch = isResponseModelMismatch(props.observation)
  const rows = [
    [t('Request Model'), props.observation.requested_model],
    [t('Upstream Model'), props.observation.upstream_model],
    [t('Response Model'), props.observation.returned_model],
  ]

  return (
    <div className='flex min-w-0 flex-col gap-2'>
      {mismatch && <ResponseModelWarning observation={props.observation} />}
      {rows.map(([label, model]) => (
        <ModelDetailRow key={label} label={label} model={model} />
      ))}
      {mismatch && (
        <p className='text-muted-foreground text-xs'>
          {t(
            'The upstream returned a different model name. This warning alone does not prove model substitution.'
          )}
        </p>
      )}
    </div>
  )
}
