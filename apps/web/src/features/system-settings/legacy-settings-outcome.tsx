/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import type { LegacySettingsBookmark } from './legacy-settings-bookmark'

export function LegacySettingsOutcome({
  category,
  section,
  titleKey,
  destination,
}: LegacySettingsBookmark) {
  const { t } = useTranslation()
  return (
    <main className='mx-auto w-full max-w-3xl space-y-5 px-6 py-10'>
      <header className='space-y-2'>
        <p className='text-muted-foreground font-mono text-xs'>
          {['system-settings', category, section].join('/')}
        </p>
        <h1 className='text-2xl font-semibold tracking-tight'>{t(titleKey)}</h1>
      </header>
      {destination ? (
        <>
          <p className='text-muted-foreground max-w-prose text-sm leading-relaxed'>
            {t(destination.detail)}
          </p>
          {destination.action === 'choose' && (
            <ul className='divide-y rounded-lg border px-4'>
              {destination.choices.map(({ href, label }) => (
                <li key={href}>
                  <a
                    href={href}
                    className='text-primary block py-3 text-sm font-medium underline-offset-4 hover:underline'
                  >
                    {t(label)}
                  </a>
                </li>
              ))}
            </ul>
          )}
        </>
      ) : (
        <p className='text-muted-foreground text-sm'>
          {t('No settings section matches this bookmark.')}
        </p>
      )}
    </main>
  )
}
