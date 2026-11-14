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
import type { SystemStatus } from '../types'
import { useBrowserAuthorization } from './use-browser-authorization'
import { useTelegramLogin } from './use-telegram-login'

/** Composes browser redirects and the separate interactive Telegram ceremony. */
export function useOAuthLogin(
  status: SystemStatus | null,
  redirectTo?: string,
  acceptedLegal = false,
  onVerifiedAccountAppeal?: () => void
) {
  const browser = useBrowserAuthorization(status, acceptedLegal)
  const telegram = useTelegramLogin(status, redirectTo, onVerifiedAccountAppeal)
  return {
    ...browser,
    ...telegram,
    isLoading: browser.isLoading || telegram.isLoading,
  }
}
