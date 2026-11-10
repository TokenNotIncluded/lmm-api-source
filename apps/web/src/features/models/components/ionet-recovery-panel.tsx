/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  pendingIoNetOperations,
  readPendingIoNetOperations,
  resumeIoNetOperation,
} from '../ionet-api'
import { deploymentsQueryKeys } from '../lib'

export function IoNetRecoveryPanel() {
  const { t } = useTranslation()
  const client = useQueryClient()
  const user = useAuthStore((state) => state.auth.user)
  const sid = useAuthStore((state) => state.auth.session?.sid)
  const [local, setLocal] = useState(() => ({
    owner: user?.id,
    rows: pendingIoNetOperations(),
  }))
  const remote = useQuery({
    queryKey: ['ionet-operations', user?.id, user?.role, sid],
    queryFn: ({ signal }) => readPendingIoNetOperations(signal),
    enabled: !!user && user.role >= ROLE.ADMIN,
  })
  const rows = [
    ...new Map(
      [
        ...(local.owner === user?.id ? local.rows : []),
        ...(remote.data ?? []),
      ].map((row) => [row.key, row])
    ).values(),
  ]
  const [busy, setBusy] = useState<string | null>(null)
  useEffect(() => {
    const refresh = () => {
      setLocal({ owner: user?.id, rows: pendingIoNetOperations() })
      void client.invalidateQueries({ queryKey: ['ionet-operations'] })
    }
    refresh()
    window.addEventListener('lmm-ionet-operations', refresh)
    window.addEventListener('storage', refresh)
    return () => {
      window.removeEventListener('lmm-ionet-operations', refresh)
      window.removeEventListener('storage', refresh)
    }
  }, [user?.id, client])
  if (!user || user.role < ROLE.ADMIN || !rows.length) return null
  const resume = async (key: string, cancel: boolean) => {
    setBusy(key)
    try {
      const receipt = await resumeIoNetOperation(key, cancel)
      if (receipt.state === 'unknown')
        toast.info(
          t(
            'The result is unknown. Resume checks the original operation without sending it again.'
          )
        )
      else {
        await Promise.all([
          client.invalidateQueries({ queryKey: deploymentsQueryKeys.lists() }),
          client.invalidateQueries({ queryKey: ['ionet-operations'] }),
        ])
        if (receipt.state === 'rejected') toast.error(t('Request failed'))
        else
          toast.success(
            t(receipt.state === 'succeeded' ? 'Success' : 'Cancelled')
          )
      }
    } catch {
      toast.error(t('Request failed'))
    } finally {
      setBusy(null)
    }
  }
  return (
    <Alert>
      <AlertTitle>
        {t('io.net Deployments')} · {t('Unknown')}
      </AlertTitle>
      <AlertDescription>
        <p>
          {t(
            'The result is unknown. Resume checks the original operation without sending it again.'
          )}
        </p>
        <div className='mt-3 space-y-3'>
          {rows.map((row) => (
            <div key={row.key} className='flex flex-wrap items-center gap-2'>
              <code className='min-w-0 break-all'>{row.key}</code>
              <Button
                variant='outline'
                size='sm'
                disabled={busy !== null || user?.role !== ROLE.SUPER_ADMIN}
                onClick={() => void resume(row.key, false)}
              >
                {t('Resume')}
              </Button>
              <Button
                variant='outline'
                size='sm'
                disabled={busy !== null || user?.role !== ROLE.SUPER_ADMIN}
                onClick={() => void resume(row.key, true)}
              >
                {t('Cancel an unstarted request')}
              </Button>
            </div>
          ))}
        </div>
      </AlertDescription>
    </Alert>
  )
}
