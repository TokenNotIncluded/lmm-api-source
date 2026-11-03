/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
export type Platform = 'linux' | 'macos' | 'windows'

export function scriptUrl(rootUrl: string | null, name: string) {
  return rootUrl ? `${rootUrl}/scripts/${encodeURIComponent(name)}` : null
}

export function commandFor(
  rootUrl: string | null,
  name: string,
  platform: Platform
) {
  const url = scriptUrl(rootUrl, name)
  if (!url) return null
  if (platform === 'windows') {
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`
    if (name.toLowerCase().endsWith('.ps1')) return `irm ${quote(url)} | iex`
    return `Invoke-WebRequest -Uri ${quote(url)} -OutFile ${quote(name)}; & ${quote(`.\\${name}`)}`
  }
  const shell = name.toLowerCase().endsWith('.zsh') ? 'zsh' : 'bash'
  return `curl -fsSL '${url.replaceAll("'", "'\\''")}' | ${shell}`
}
