/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */

// Decorative only: recovery actions and the readable status belong to the frame.
const marks: Record<string, string> = {
  '401': 'M28 21a9 9 0 1 0-7 7L9 40H3v-6l12-12 M28 14h.01',
  '403': 'M24 4 40 11v12c0 10-7 17-16 21C15 40 8 33 8 23V11Z M17 24h14',
  '404': 'M17 17a7 7 0 1 1 12 5c-4 3-5 4-5 9 M24 38h.01',
  '429': 'M17 14v20 M31 14v20 M8 8a23 23 0 1 1-5 24 M3 7v9h9',
  '503':
    'M31 5a12 12 0 0 0-14 16L5 33a7 7 0 0 0 10 10l12-12A12 12 0 0 0 43 17l-8 8-12-12Z',
}
const disconnected =
  'M16 4v10 M32 4v10 M12 14h24v5a12 12 0 0 1-24 0Z M24 31v5 M18 42l6-6 6 6'

export function ErrorArtwork({ status }: { status?: string | number }) {
  return (
    <svg
      className='error-editorial-art'
      viewBox='0 0 440 320'
      fill='none'
      aria-hidden='true'
      focusable='false'
    >
      <circle className='error-art-halo' cx='220' cy='160' r='102' />
      <g className='error-art-guides'>
        <path d='M48 84V68h16m312 0h16v16M48 236v16h16m312 0h16v-16' />
        <path d='M212 68h16m-16 184h16' />
      </g>
      <path className='error-art-trace' d='M60 160h120' />
      <path className='error-art-disconnected' d='M260 160h120' />
      <circle className='error-art-terminal' cx='60' cy='160' r='7' />
      <circle className='error-art-endpoint' cx='380' cy='160' r='7' />
      <rect
        className='error-art-core'
        x='180'
        y='120'
        width='80'
        height='80'
        rx='24'
      />
      <svg x='200' y='140' width='40' height='40' viewBox='0 0 48 48'>
        <path
          className='error-art-symbol'
          d={marks[String(status)] ?? disconnected}
        />
      </svg>
    </svg>
  )
}
