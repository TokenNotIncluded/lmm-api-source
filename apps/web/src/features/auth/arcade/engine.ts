/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
export function shuffle<T>(items: T[]): T[] {
  const result = [...items]
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[result[i], result[j]] = [result[j], result[i]]
  }
  return result
}

export function slideTile(board: number[], index: number): number[] {
  const empty = board.indexOf(0)
  if (
    Math.abs(Math.floor(empty / 3) - Math.floor(index / 3)) +
      Math.abs((empty % 3) - (index % 3)) !==
    1
  ) {
    return board
  }
  const next = [...board]
  ;[next[empty], next[index]] = [next[index], next[empty]]
  return next
}

export function solvedSlide(board: number[]): boolean {
  return board.every((tile, index) => tile === (index + 1) % 9)
}

export function newSlide(): number[] {
  let board = [1, 2, 3, 4, 5, 6, 7, 8, 0]
  let previous = -1
  for (let step = 0; step < 80; step++) {
    const empty = board.indexOf(0)
    const choices = board
      .map((_, i) => i)
      .filter((i) => i !== previous && slideTile(board, i) !== board)
    board = slideTile(
      board,
      choices[Math.floor(Math.random() * choices.length)]
    )
    previous = empty
  }
  return solvedSlide(board) ? slideTile(board, 7) : board
}

export function toggleLights(board: boolean[], index: number): boolean[] {
  return board.map((on, i) =>
    Math.abs(Math.floor(i / 4) - Math.floor(index / 4)) +
      Math.abs((i % 4) - (index % 4)) <=
    1
      ? !on
      : on
  )
}

export function newLights(): boolean[] {
  let board = Array<boolean>(16).fill(false)
  for (const index of shuffle(Array.from({ length: 16 }, (_, i) => i)).slice(
    0,
    7
  )) {
    board = toggleLights(board, index)
  }
  return board.some(Boolean) ? board : toggleLights(board, 5)
}
