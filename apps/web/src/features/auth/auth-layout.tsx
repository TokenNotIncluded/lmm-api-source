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
import { Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { LanguageSwitcher } from '@/components/language-switcher'
import { BackToConsole } from '@/components/layout/back-to-console'
import { LmmBrandMark } from '@/components/lmm-brand-mark'
import { ThemeSwitch } from '@/components/theme-switch'
import { useSystemConfig } from '@/hooks/use-system-config'
import { useAuthStore } from '@/stores/auth-store'

import { AuthArcade } from './arcade/arcade'

import './auth-mobile.css'

type AuthLayoutProps = {
  children: React.ReactNode
  reloadDocument?: boolean
  allowConsoleReturn?: boolean
}

export function AuthLayout({
  children,
  reloadDocument,
  allowConsoleReturn = false,
}: AuthLayoutProps) {
  const { systemName } = useSystemConfig()
  const { t } = useTranslation()
  const signedIn = useAuthStore(({ auth }) =>
    Boolean(auth.user && auth.accessToken)
  )
  const [gameOpen, setGameOpen] = useState(false)
  useEffect(() => {
    const previousTitle = document.title
    document.title = systemName
    return () => {
      document.title = previousTitle
    }
  }, [systemName])

  return (
    <div className='auth-editorial flex min-h-dvh min-w-0 flex-col'>
      <header className='auth-header flex min-w-0 shrink-0 flex-wrap items-center justify-between'>
        <div className='flex min-w-0 flex-wrap items-center gap-x-4'>
          {allowConsoleReturn && signedIn && <BackToConsole />}
          <Link
            to='/'
            reloadDocument={reloadDocument}
            className='auth-brand flex min-w-0 items-center gap-2.5 transition-opacity hover:opacity-80'
          >
            <LmmBrandMark className='size-7 shrink-0' title={systemName} />
            <h1 className='min-w-0 text-sm font-semibold tracking-tight wrap-anywhere'>
              {systemName}
            </h1>
          </Link>
        </div>
        <div className='auth-preferences text-muted-foreground flex shrink-0 items-center gap-1'>
          <LanguageSwitcher />
          <ThemeSwitch />
        </div>
      </header>
      <main className='auth-main flex min-w-0 flex-1 flex-col items-center'>
        <div className='auth-form-stage w-full max-w-md'>{children}</div>
      </main>
      <footer className='auth-footer'>
        <details
          className='auth-game-disclosure'
          onToggle={(event) => setGameOpen(event.currentTarget.open)}
        >
          <summary>{t('Play a round')}</summary>
          {gameOpen && <AuthArcade />}
        </details>
      </footer>
    </div>
  )
}
