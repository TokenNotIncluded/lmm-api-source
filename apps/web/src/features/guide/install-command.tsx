/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
/*
Copyright (C) 2026 LIghtJUNction
*/
import { Check, Copy } from 'lucide-react'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useCopyToClipboard } from '@/hooks/use-copy-to-clipboard'

export function InstallCommand({
  value,
  copyLabel,
}: {
  value: string
  copyLabel: string
}) {
  const { t } = useTranslation()
  const { copiedText, copyToClipboard } = useCopyToClipboard({ notify: false })
  const [failedValue, setFailedValue] = useState<string | null>(null)
  const command = useRef<HTMLElement>(null)
  const copied = copiedText === value

  const copy = async () => {
    const success = await copyToClipboard(value)
    setFailedValue(success ? null : value)
    if (!success && command.current) {
      command.current.focus()
      window.getSelection()?.selectAllChildren(command.current)
    }
  }

  return (
    <>
      <div className='flex min-w-0 flex-wrap items-start gap-2'>
        <code
          ref={command}
          tabIndex={0}
          className='bg-muted focus-visible:outline-ring block min-h-11 min-w-0 flex-[1_1_18rem] rounded-lg px-3 py-3 text-xs leading-6 wrap-anywhere whitespace-pre-wrap select-text focus-visible:outline-2 focus-visible:outline-offset-2'
        >
          {value}
        </code>
        <Button
          type='button'
          variant='outline'
          size='sm'
          onClick={() => void copy()}
          aria-label={copyLabel}
          className='min-h-11 shrink-0'
        >
          {copied ? (
            <Check data-icon='inline-start' />
          ) : (
            <Copy data-icon='inline-start' />
          )}
          {copied ? t('Copied') : t('Copy')}
        </Button>
      </div>
      <span role='status' className='sr-only'>
        {copied ? t('Copied') : ''}
      </span>
      {failedValue === value && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Copy failed. The code is selected for manual copying.')}
        </p>
      )}
    </>
  )
}
