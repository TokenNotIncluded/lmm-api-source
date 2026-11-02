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
import {
  isHeaderNavAccessModule,
  type HeaderNavModules,
} from './header-nav-config'

export type TopNavSurface = 'default' | 'public'

export type TopNavLink = {
  title: string
  href: string
  disabled?: boolean
  requiresAuth?: boolean
  external?: boolean
}

// Domain routes and translation keys live here; shells only select a surface.
const ROUTES = {
  home: { title: 'Home', href: '/' },
  directory: { title: 'AI directory', href: '/ai-directory' },
  console: { title: 'Open workspace', href: '' },
  pricing: { title: 'Models and pricing', href: '/pricing' },
  developers: { title: 'Developers', href: '/developers' },
  guide: { title: 'Guide', href: '/guide' },
  scripts: { title: 'Scripts', href: '/scripts' },
  webmcp: { title: 'WebMCP', href: '/webmcp' },
  signal: { title: 'Signal path', href: '/games/signal' },
  challenges: { title: 'Challenges', href: '/challenges' },
  rankings: { title: 'Rankings', href: '/rankings' },
  security: { title: 'Security', href: '/security' },
  docs: { title: 'Docs', href: '' },
  about: { title: 'About', href: '/about' },
} as const

const SURFACE_ORDER: Record<TopNavSurface, readonly (keyof typeof ROUTES)[]> = {
  default: [
    'home',
    'console',
    'pricing',
    'scripts',
    'developers',
    'webmcp',
    'signal',
    'rankings',
    'security',
    'docs',
    'about',
  ],
  public: [
    'home',
    'directory',
    'pricing',
    'developers',
    'guide',
    'scripts',
    'webmcp',
    'signal',
    'challenges',
    'security',
  ],
}

export function buildTopNavLinks({
  modules,
  surface = 'default',
  isAuthed,
  workspaceHref,
  hasDocsAccess,
  docsLink,
  translate,
}: {
  modules: HeaderNavModules
  surface?: TopNavSurface
  isAuthed: boolean
  workspaceHref: string
  hasDocsAccess: boolean
  docsLink?: string
  translate: (key: string) => string
}): TopNavLink[] {
  const links: TopNavLink[] = []
  for (const key of SURFACE_ORDER[surface]) {
    const route = ROUTES[key]
    const title = key === 'webmcp' ? route.title : translate(route.title)
    if (isHeaderNavAccessModule(key)) {
      const access = modules[key]
      if (access.enabled) {
        links.push({
          title,
          href: route.href,
          requiresAuth: access.requireAuth && !isAuthed,
        })
      }
      continue
    }
    if (modules[key] !== true) continue
    if (key === 'console') {
      if (isAuthed) links.push({ title, href: workspaceHref })
    } else if (key === 'docs') {
      if (hasDocsAccess && docsLink) {
        links.push({ title, href: docsLink, external: true })
      }
    } else {
      links.push({ title, href: route.href })
    }
  }
  return links
}
