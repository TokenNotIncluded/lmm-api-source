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
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { SparklesIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Dialog } from '@/components/dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Markdown } from '@/components/ui/markdown'
import { formatTimestampToDate } from '@/lib/format'
import { useAuthStore } from '@/stores/auth-store'

import { getLatestUnreadReleaseNote, markReleaseNoteRead } from './api'

export function ReleaseNoteDialog() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const user = useAuthStore((state) => state.auth.user)
  const session = useAuthStore((state) => state.auth.session)
  const queryKey = [
    'release-notes',
    'latest-unread',
    user?.id ?? 0,
    session?.sid ?? 'authenticated',
  ] as const

  const releaseNoteQuery = useQuery({
    queryKey,
    queryFn: getLatestUnreadReleaseNote,
    enabled: Boolean(user?.id),
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
  })
  const releaseNote = releaseNoteQuery.data ?? null

  const acknowledgeMutation = useMutation({
    mutationFn: async () => {
      if (!releaseNote) return
      await markReleaseNoteRead(releaseNote.id)
    },
    onSuccess: () => {
      queryClient.setQueryData(queryKey, null)
    },
    onError: (error: Error) => {
      toast.error(error.message || t('Failed to acknowledge release note'))
    },
  })

  if (!releaseNote) return null

  return (
    <Dialog
      open
      onOpenChange={() => undefined}
      showCloseButton={false}
      title={t("What's new in {{version}}", {
        version: releaseNote.version,
      })}
      description={t('Published {{date}}', {
        date: formatTimestampToDate(releaseNote.published_at),
      })}
      contentClassName='sm:max-w-3xl'
      contentHeight='min(55vh, 34rem)'
      bodyClassName='space-y-4'
      footer={
        <Button
          type='button'
          disabled={acknowledgeMutation.isPending}
          onClick={() => acknowledgeMutation.mutate()}
        >
          {acknowledgeMutation.isPending
            ? t('Saving...')
            : t('Got it, continue')}
        </Button>
      }
    >
      <div className='bg-primary/5 border-primary/15 flex items-center gap-3 border p-3'>
        <div className='bg-primary/10 text-primary flex size-9 shrink-0 items-center justify-center rounded-full'>
          <SparklesIcon className='size-4' />
        </div>
        <div className='min-w-0'>
          <div className='flex flex-wrap items-center gap-2 font-medium'>
            <span>{releaseNote.version}</span>
            {releaseNote.revision > 1 && (
              <Badge variant='secondary'>
                {t('Revision {{revision}}', {
                  revision: releaseNote.revision,
                })}
              </Badge>
            )}
          </div>
          <p className='text-muted-foreground text-sm'>
            {t('This update will only be shown once after you acknowledge it.')}
          </p>
        </div>
      </div>
      <Markdown className='release-note-markdown'>
        {releaseNote.content}
      </Markdown>
    </Dialog>
  )
}
