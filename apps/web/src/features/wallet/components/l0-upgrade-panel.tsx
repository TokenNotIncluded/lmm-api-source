/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

export function L0UpgradePanel({ preview = false }: { preview?: boolean }) {
  const { t } = useTranslation()
  return (
    <section
      className='bg-primary/5 rounded-2xl p-6 sm:p-8'
      aria-labelledby='l0-wallet-title'
    >
      <p className='text-primary text-xs font-semibold tracking-wide'>
        {t('L0 · Get started')}
      </p>
      <h2
        id='l0-wallet-title'
        className='mt-2 text-xl font-semibold tracking-tight sm:text-2xl'
      >
        {t('Your next idea starts here')}
      </h2>
      <p className='text-muted-foreground mt-3 max-w-2xl text-sm leading-6'>
        {t(
          'Explore models, compare prices, then choose how much to add. Review your quote before confirming payment.'
        )}
      </p>
      <div className='mt-5 flex flex-wrap gap-3'>
        <Button
          disabled={preview}
          onClick={() =>
            document
              .getElementById('wallet-checkout')
              ?.scrollIntoView({ block: 'start', behavior: 'auto' })
          }
        >
          {t('Add funds')}
        </Button>
        {preview ? (
          <Button variant='outline' disabled>
            {t('Browse models')}
          </Button>
        ) : (
          <Button variant='outline' render={<Link to='/pricing' />}>
            {t('Browse models')}
          </Button>
        )}
      </div>
      <p className='text-muted-foreground mt-4 text-xs leading-5'>
        {t(
          'Activation follows your account eligibility. Adding funds does not bypass access rules.'
        )}
      </p>
    </section>
  )
}
