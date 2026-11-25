import { useQuery } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { SectionPageLayout } from '@/components/layout'
import { Button } from '@/components/ui/button'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { readSSMCPDSelf, useSSMCPDCapabilities } from './ssmcpd-capabilities'
import { SSMCPDCommerce } from './ssmcpd-commerce'

const bytes = z.string().regex(/^(0|[1-9]\d*)$/)
const quotaSchema = z.object({
  username: z.string(),
  period: z.string(),
  uploaded_bytes: bytes,
  downloaded_bytes: bytes,
  used_bytes: bytes,
  remaining_bytes: bytes.nullable(),
  limit_bytes: bytes.nullable(),
  reset_at: z.number().int().positive(),
})

export function SSMCPDPage() {
  const { t, i18n } = useTranslation()
  const auth = useAuthStore((s) => s.auth)
  const userId = auth.user?.id
  const sessionId = auth.session?.sid
  const admin = (auth.user?.role ?? 0) >= ROLE.ADMIN
  const ready = !!userId && !!sessionId
  const scope = { userId: userId ?? 0, sessionId: sessionId ?? '' }
  const capabilities = useSSMCPDCapabilities()
  const info = capabilities.data
  const usage = useQuery({
    queryKey: ['ssmcpd', userId, sessionId, 'usage'],
    enabled: ready && info?.account_usage_api_configured === true,
    retry: false,
    gcTime: 0,
    queryFn: async ({ signal }) =>
      quotaSchema.parse(
        await readSSMCPDSelf('/api/plugins/ssmcpd/usage', scope, signal)
      ),
  })
  const quota = usage.data
  const portal = info?.service.traffic_portal
  const publicOrigin = info?.service.public_origin
  const portalLink =
    portal &&
    publicOrigin?.startsWith('https://') &&
    portal === publicOrigin + '/account/traffic'
      ? portal
      : undefined
  const unbound =
    isAxiosError(usage.error) &&
    usage.error.response?.data?.code === 'SSMCPD_ACCOUNT_UNBOUND'
  const byteValue = (value: string | null) =>
    value === null
      ? t('Unlimited')
      : BigInt(value).toLocaleString(i18n.language) + ' B'
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>SSMCP</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        <div className='max-w-2xl space-y-8'>
          {capabilities.isPending && <p role='status'>{t('Loading…')}</p>}
          {capabilities.isError && (
            <div role='alert'>
              {t('Unable to load plugin configuration.')}{' '}
              <Button
                variant='outline'
                onClick={() => void capabilities.refetch()}
              >
                {t('Retry')}
              </Button>
            </div>
          )}
          {info && (
            <>
              <section className='space-y-2'>
                <h2 className='font-medium'>{t('Connection command')}</h2>
                <code className='bg-muted block overflow-x-auto rounded-md p-3'>
                  {info.service.connect_command}
                </code>
                {admin && (
                  <p className='text-muted-foreground text-sm'>
                    {t('Mode')} · {info.service.mode}
                  </p>
                )}
              </section>
              <section className='space-y-3'>
                <h2 className='font-medium'>{t('Usage')}</h2>
                {info.account_usage_api_configured ? (
                  <>
                    {usage.isPending && <p role='status'>{t('Loading…')}</p>}
                    {usage.isError && (
                      <div role='alert'>
                        {unbound
                          ? t('No SSMCP account is linked.')
                          : t('SSMCP usage could not be loaded.')}{' '}
                        <Button
                          variant='outline'
                          onClick={() => void usage.refetch()}
                        >
                          {t('Retry')}
                        </Button>
                      </div>
                    )}
                    {quota && !usage.isError && (
                      <dl className='grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-2 text-sm'>
                        {[
                          [t('Username'), quota.username],
                          [t('Period'), quota.period],
                          [t('Upload'), byteValue(quota.uploaded_bytes)],
                          [t('Download'), byteValue(quota.downloaded_bytes)],
                          [t('Used'), byteValue(quota.used_bytes)],
                          [t('Remaining'), byteValue(quota.remaining_bytes)],
                          [
                            t('Next reset'),
                            new Date(quota.reset_at * 1000).toLocaleString(
                              i18n.language
                            ),
                          ],
                        ].map(([label, value]) => (
                          <div key={label} className='contents'>
                            <dt className='text-muted-foreground'>{label}</dt>
                            <dd className='text-end tabular-nums'>{value}</dd>
                          </div>
                        ))}
                      </dl>
                    )}
                  </>
                ) : (
                  !portalLink && <p>{t('SSMCP metering is not configured.')}</p>
                )}
                {portalLink && (
                  <a
                    className='underline underline-offset-4'
                    href={portalLink}
                    rel='noreferrer'
                    target='_blank'
                  >
                    {t('Open traffic portal')}
                  </a>
                )}
              </section>
              {info.credit_grant_configured && userId && sessionId ? (
                <SSMCPDCommerce
                  key={userId + ':' + sessionId}
                  userId={userId}
                  sessionId={sessionId}
                />
              ) : null}
            </>
          )}
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
