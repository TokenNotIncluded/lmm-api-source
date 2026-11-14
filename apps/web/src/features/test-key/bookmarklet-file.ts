/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { buildTestKeyBookmarklet, TEST_KEY_BOOKMARK_NAME } from './bookmarklet'

// Rasterized from public/test-key-mark.svg at 32px. Keep this self-contained
// PNG in sync with the popup mark: bookmark importers cannot use SVG icons.
export const TEST_KEY_BOOKMARK_ICON =
  'data:image/png;base64,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'

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
}

/** Importing this file saves the script and icon, never an account credential. */
export function buildTestKeyBookmarkFile(
  origin: string,
  name = TEST_KEY_BOOKMARK_NAME
): string {
  const title = escapeHtml(name)
  const href = escapeHtml(buildTestKeyBookmarklet(origin))
  return [
    '<!DOCTYPE NETSCAPE-Bookmark-file-1>',
    '<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">',
    `<TITLE>${title}</TITLE>`,
    `<H1>${title}</H1>`,
    '<DL><p>',
    `    <DT><A HREF="${href}" ICON="${TEST_KEY_BOOKMARK_ICON}">${title}</A>`,
    '</DL><p>',
    '',
  ].join('\n')
}

export function testKeyBookmarkDownloadUrl(
  origin: string,
  name: string
): string {
  return `data:text/html;charset=utf-8,${encodeURIComponent(buildTestKeyBookmarkFile(origin, name))}`
}
