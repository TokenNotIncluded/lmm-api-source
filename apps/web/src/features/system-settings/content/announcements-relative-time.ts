/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { normalizeInterfaceLanguage } from '@/i18n/languages'

export function announcementRelativeTime(
  date: string,
  now: number,
  language: string
): string {
  const delta = new Date(date).getTime() - now
  if (!Number.isFinite(delta)) return date
  const magnitude = Math.abs(delta)
  const [divisor, unit] =
    magnitude < 3_600_000
      ? ([60_000, 'minute'] as const)
      : magnitude < 86_400_000
        ? ([3_600_000, 'hour'] as const)
        : ([86_400_000, 'day'] as const)
  return new Intl.RelativeTimeFormat(normalizeInterfaceLanguage(language), {
    numeric: 'auto',
  }).format(Math.round(delta / divisor), unit)
}
