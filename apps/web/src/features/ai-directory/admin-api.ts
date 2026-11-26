/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { api } from '@/lib/api'

import { parseDirectoryLinks, type AIDirectoryLink } from './api'
import { getPublicDirectory } from './public-api'

type DirectoryEnvelope = {
  success: boolean
  message?: string
  data?: { links?: unknown }
}

function readLinks(envelope: DirectoryEnvelope): AIDirectoryLink[] | null {
  if (envelope.success !== true) {
    throw new Error(envelope.message || 'Unable to load AI directory')
  }
  const links = envelope.data?.links
  if (links === null) return null
  const parsed =
    Array.isArray(links) && parseDirectoryLinks(JSON.stringify(links))
  if (!parsed) throw new Error('Unable to load AI directory')
  return parsed
}

export async function getDirectorySettings() {
  const { data } = await getPublicDirectory<DirectoryEnvelope>(
    '/api/ai-directory',
    'no-store'
  )
  return readLinks(data)
}

export function sameDirectoryLinks(
  left: AIDirectoryLink[],
  right: AIDirectoryLink[]
) {
  return (
    left.length === right.length &&
    left.every((link, index) => {
      const other = right[index]
      return (
        link.id === other.id &&
        link.name === other.name &&
        link.url === other.url &&
        link.category === other.category &&
        link.summary === other.summary &&
        link.description === other.description &&
        link.enabled === other.enabled
      )
    })
  )
}

export async function saveDirectorySettings(links: AIDirectoryLink[]) {
  const { data } = await api.put<DirectoryEnvelope>(
    '/api/ai-directory/links',
    { links },
    {
      skipErrorHandler: true,
      skipBusinessError: true,
    }
  )
  const saved = readLinks(data)
  if (saved === null || !sameDirectoryLinks(saved, links)) {
    throw new Error('Failed to save')
  }
  const current = await getDirectorySettings()
  if (current === null || !sameDirectoryLinks(current, links)) {
    throw new Error('Failed to save')
  }
  return current
}
