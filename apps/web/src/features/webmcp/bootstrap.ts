/*
Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
*/
import type { WebMcpRouter } from './tool-kit'

/** Ordinary browsers never download the optional agent tool catalogue. */
export async function bootstrapWebMcp(
  router: WebMcpRouter
): Promise<(() => void) | undefined> {
  try {
    if (
      typeof document === 'undefined' ||
      typeof document.modelContext?.registerTool !== 'function'
    ) {
      return undefined
    }
    const { installWebMcp } = await import('./index')
    return installWebMcp(router)
  } catch {
    // A blocked capability getter or unavailable chunk must not prevent startup.
    return undefined
  }
}
