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
import { LmmBrandMark } from '@/components/lmm-brand-mark'
import { ThemeSwitch } from '@/components/theme-switch'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useSystemConfig } from '@/hooks/use-system-config'

import { AuthArcade } from './arcade/arcade'

import '../forge/forge-public-shell.css'
import './auth-mobile.css'

type AuthLayoutProps = {
  children: React.ReactNode
  reloadDocument?: boolean
}

export function AuthLayout({ children, reloadDocument }: AuthLayoutProps) {
  const { systemName } = useSystemConfig()
  const { t } = useTranslation()
  const desktop = useMediaQuery('(min-width: 1024px)')
  const [mobileGameOpen, setMobileGameOpen] = useState(false)
  useEffect(() => {
    const previousTitle = document.title
    document.title = systemName
    return () => {
      document.title = previousTitle
    }
  }, [systemName])

  return (
    <div className='auth-editorial relative flex h-dvh max-w-none flex-col overflow-hidden lg:grid lg:grid-cols-2'>
      <header className='auth-header relative z-20 flex min-h-16 shrink-0 items-center justify-between px-5 sm:min-h-20 sm:px-8 lg:absolute lg:top-0 lg:left-0 lg:min-h-24 lg:w-1/2 lg:px-10'>
        <Link
          to='/'
          reloadDocument={reloadDocument}
          className='flex min-w-0 items-center gap-2.5 transition-opacity hover:opacity-80'
        >
          <LmmBrandMark className='size-8' title={systemName} />
          <h1 className='truncate text-base font-semibold tracking-tight sm:text-lg'>
            {systemName}
          </h1>
        </Link>
        <div className='flex shrink-0 items-center gap-1'>
          <LanguageSwitcher />
          <ThemeSwitch />
        </div>
      </header>
      <div className='grid min-h-0 flex-1 lg:col-start-1 lg:h-full'>
        <div className='no-scrollbar container min-h-0 overflow-y-auto lg:pt-24'>
          <div className='auth-form-stage mx-auto flex min-h-full w-full max-w-md flex-col justify-center px-5 py-10 sm:px-8 sm:py-14'>
            {children}
            <details
              className='auth-game-disclosure mt-8 w-full lg:hidden'
              onToggle={(event) => setMobileGameOpen(event.currentTarget.open)}
            >
              <summary>{t('Play a round')}</summary>
              {!desktop && mobileGameOpen && <AuthArcade />}
            </details>
          </div>
        </div>
      </div>
      <div className='auth-art-stage hidden lg:col-start-2 lg:row-start-1 lg:flex lg:h-dvh lg:min-h-0 lg:flex-col lg:overflow-y-auto'>
        {desktop && <AuthArcade />}
      </div>
    </div>
  )
}
