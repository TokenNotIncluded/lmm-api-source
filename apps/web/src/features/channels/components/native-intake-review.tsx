/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { getServerErrorMessage } from '@/lib/server-error-message'

import type {
  NativeConfiguration,
  NativeRoutingScope,
  NativeUpstream,
} from '../native-api'
import {
  emptyLimits,
  groupIntake,
  inputRequirements,
  intakeIndex,
  publishIntakeRoutes,
  reviewIntakeRoutes,
  type IntakeLimits,
  type IntakeModel,
} from './native-intake-routes'

export function NativeIntakeReview({
  scope,
  configuration,
  channel,
  models,
  onBusy,
  onRefresh,
  onClose,
}: {
  scope: NativeRoutingScope
  configuration: NativeConfiguration
  channel: NativeUpstream
  models: IntakeModel[]
  onBusy: (value: boolean) => void
  onRefresh: () => Promise<void>
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [groups, setGroups] = useState<string[]>([])
  const [limits, setLimits] = useState<Record<string, IntakeLimits>>({})
  const modelLimits = (id: string) =>
    Object.hasOwn(limits, id) ? limits[id] : emptyLimits
  const [review, setReview] =
    useState<Awaited<ReturnType<typeof reviewIntakeRoutes>>>()
  const [busy, setBusy] = useState(false)
  const [published, setPublished] = useState(false)
  const [error, setError] = useState('')
  const lock = useRef(false)
  const index = useMemo(() => intakeIndex(configuration), [configuration])
  const fields = inputRequirements(channel)
  const states = models.flatMap((pair) =>
    groups.map((group) => ({
      pair,
      group,
      state: groupIntake(
        configuration,
        index,
        channel,
        pair,
        group,
        modelLimits(pair.upstream)
      ),
    }))
  )
  const duplicate = new Set(models.map((p) => p.model)).size !== models.length
  const available =
    states.length > 0 &&
    !duplicate &&
    states.some((row) => !row.state.done) &&
    states.every((row) => !row.state.reasons.length)
  function reset() {
    setReview(undefined)
    setError('')
  }
  function changeLimit(
    model: string,
    field: keyof IntakeLimits,
    value: string
  ) {
    reset()
    setLimits({ ...limits, [model]: { ...modelLimits(model), [field]: value } })
  }
  async function submit(publish: boolean) {
    if (lock.current || published || !available || (publish && !review)) return
    lock.current = true
    setBusy(true)
    onBusy(true)
    setError('')
    try {
      if (publish && review) {
        await publishIntakeRoutes(scope, review)
        setPublished(true)
        setReview(undefined)
        await onRefresh()
        onClose()
      } else {
        setReview(
          await reviewIntakeRoutes(
            scope,
            configuration,
            channel.id,
            models,
            groups,
            limits
          )
        )
      }
    } catch (cause) {
      setReview(undefined)
      setError(
        t(
          getServerErrorMessage(cause) ??
            (cause instanceof Error ? cause.message : 'Failed to save')
        )
      )
    } finally {
      lock.current = false
      onBusy(false)
      setBusy(false)
    }
  }
  return (
    <section
      aria-label={t('Review selected routes')}
      className='space-y-4 border-t pt-4'
    >
      <fieldset disabled={busy || published} className='space-y-3'>
        <legend className='text-sm font-medium'>
          {t('Routing groups for the selected models')}
        </legend>
        <div className='flex flex-wrap gap-x-4 gap-y-2'>
          {configuration.routing.groups
            .filter((g) => g.enabled)
            .map(({ name }) => (
              <label key={name} className='flex items-center gap-2 text-sm'>
                <input
                  type='checkbox'
                  checked={groups.includes(name)}
                  onChange={(e) => {
                    reset()
                    setGroups(
                      e.target.checked
                        ? [...groups, name]
                        : groups.filter((g) => g !== name)
                    )
                  }}
                />
                {name}
              </label>
            ))}
        </div>
        {!index.groups.size && (
          <p role='status'>{t('Enable a routing group before publishing.')}</p>
        )}
        {models
          .filter((pair) =>
            states.some(
              (row) =>
                row.pair === pair && !row.state.context && !row.state.done
            )
          )
          .map((pair) => (
            <div
              key={pair.upstream}
              className='grid gap-2 text-sm sm:grid-cols-3'
            >
              <p className='font-mono break-all sm:col-span-3'>
                {pair.upstream}
              </p>
              {(
                [
                  ...(fields.input
                    ? [['input', t('Maximum input tokens')]]
                    : []),
                  ...(fields.output
                    ? [['output', t('Maximum output tokens')]]
                    : []),
                  ['revision', t('Contract revision')],
                ] as Array<[keyof IntakeLimits, string]>
              ).map(([field, label]) => (
                <label key={field} className='grid gap-1'>
                  {label}
                  <Input
                    type={field === 'revision' ? 'text' : 'number'}
                    min={field === 'revision' ? undefined : 1}
                    aria-label={pair.upstream + ' · ' + label}
                    value={modelLimits(pair.upstream)[field]}
                    onChange={(e) =>
                      changeLimit(pair.upstream, field, e.target.value)
                    }
                  />
                </label>
              ))}
            </div>
          ))}
      </fieldset>
      {duplicate && (
        <p role='alert'>
          {t('Each selected upstream model needs a unique public model ID.')}
        </p>
      )}
      {!review && states.length > 0 && (
        <ul className='divide-y text-sm'>
          {states.map(({ pair, group, state }) => (
            <li key={JSON.stringify([pair.upstream, group])} className='py-2'>
              <strong className='font-medium'>
                {pair.model} · {group}
              </strong>
              <span className='ms-2'>
                {state.reasons.length
                  ? state.reasons.map((reason) => t(reason)).join(' · ')
                  : state.done
                    ? t('Already published')
                    : t('Ready for route preview')}
              </span>
            </li>
          ))}
        </ul>
      )}
      {review && (
        <div className='space-y-3' role='status'>
          <p className='text-sm'>
            {t('Routing version')} {configuration.version.routing} →{' '}
            {review.body.routing.version}
            {' · '}
            {t('Pricing version')} {configuration.version.pricing} →{' '}
            {review.body.prices.version}
          </p>
          <ul className='divide-y text-sm'>
            {review.summaries.map((row) => (
              <li key={JSON.stringify([row.model, row.group])} className='py-2'>
                {row.model} · {row.group} · {t('Affected channels')}{' '}
                {row.channels.map((id) => '#' + id).join(', ')}
              </li>
            ))}
          </ul>
        </div>
      )}
      {error && (
        <div role='alert' className='space-y-1 text-sm'>
          <p className='text-destructive'>{error}</p>
          <Button
            type='button'
            variant='ghost'
            disabled={busy}
            onClick={() => {
              setReview(undefined)
              void onRefresh().then(
                () => setError(''),
                (cause) =>
                  setError(
                    getServerErrorMessage(cause) ?? t('Failed to load data')
                  )
              )
            }}
          >
            {t('Reload')}
          </Button>
        </div>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          disabled={busy || published || !available}
          onClick={() => void submit(false)}
        >
          {t('Preview selected routes')}
        </Button>
        <Button
          type='button'
          disabled={busy || !review || published || !available}
          onClick={() => void submit(true)}
        >
          {t('Publish selected routes')}
        </Button>
        <Button type='button' variant='ghost' disabled={busy} onClick={onClose}>
          {t('Cancel')}
        </Button>
      </div>
    </section>
  )
}
