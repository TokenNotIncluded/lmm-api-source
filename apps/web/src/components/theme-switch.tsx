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
import { Check, ChevronDown, Moon, Sun } from 'lucide-react'
import { useCallback, useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { THEME_COLORS } from '@/context/theme'
import { useTheme } from '@/context/theme-provider'
import { cn } from '@/lib/utils'

export function ThemeSwitch() {
  const { t } = useTranslation()
  const { resolvedTheme, theme, setTheme } = useTheme()

  const toggleTheme = useCallback(() => {
    setTheme(resolvedTheme === 'dark' ? 'light' : 'dark')
  }, [resolvedTheme, setTheme])

  const selectLightTheme = useCallback(() => setTheme('light'), [setTheme])
  const selectDarkTheme = useCallback(() => setTheme('dark'), [setTheme])
  const selectSystemTheme = useCallback(() => setTheme('system'), [setTheme])

  /* Update theme-color meta tag
   * when theme is updated */
  useEffect(() => {
    const themeColor = THEME_COLORS[resolvedTheme]
    const metaThemeColor = document.querySelector("meta[name='theme-color']")
    if (metaThemeColor) metaThemeColor.setAttribute('content', themeColor)
  }, [resolvedTheme])

  return (
    <div className='flex items-center'>
      <Button
        variant='ghost'
        size='icon'
        className='h-9 w-9'
        onClick={toggleTheme}
        aria-label={t('Toggle theme')}
      >
        <Sun className='size-[1.2rem] scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90' />
        <Moon className='absolute size-[1.2rem] scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0' />
      </Button>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger
          render={
            <Button
              variant='ghost'
              size='icon'
              className='h-9 w-6'
              aria-label={t('Theme options')}
            />
          }
        >
          <ChevronDown className='size-3.5' aria-hidden='true' />
        </DropdownMenuTrigger>
        <DropdownMenuContent align='end'>
          <DropdownMenuItem onClick={selectLightTheme}>
            {t('Light')}{' '}
            <Check
              size={14}
              className={cn('ms-auto', theme !== 'light' && 'hidden')}
            />
          </DropdownMenuItem>
          <DropdownMenuItem onClick={selectDarkTheme}>
            {t('Dark')}
            <Check
              size={14}
              className={cn('ms-auto', theme !== 'dark' && 'hidden')}
            />
          </DropdownMenuItem>
          <DropdownMenuItem onClick={selectSystemTheme}>
            {t('System')}
            <Check
              size={14}
              className={cn('ms-auto', theme !== 'system' && 'hidden')}
            />
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
