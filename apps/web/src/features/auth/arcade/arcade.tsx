/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  ArrowUpRight,
  CircuitBoard,
  Grid2X2,
  Lightbulb,
  RotateCcw,
  Sparkles,
  Layers,
  Check,
} from 'lucide-react'
/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { SignalGamePanel } from '@/features/signal-game/panel'

import {
  newLights,
  newSlide,
  shuffle,
  slideTile,
  solvedSlide,
  toggleLights,
} from './engine'

import './arcade.css'

export function AuthArcade() {
  const { t } = useTranslation()
  const [game, setGame] = useState('signal')
  const id = useId()
  const games = [
    { id: 'signal', name: t('Signal path'), icon: CircuitBoard },
    { id: 'memory', name: t('Memory pairs'), icon: Layers },
    { id: 'slide', name: t('Sliding numbers'), icon: Grid2X2 },
    { id: 'lights', name: t('Lights out'), icon: Lightbulb },
  ]
  return (
    <section className='auth-arcade' aria-label={t('Take a little break')}>
      <div className='arcade-intro'>
        <span>
          <Sparkles size={14} aria-hidden='true' />
          {t('Take a little break')}
        </span>
        <span>01 — 04</span>
      </div>
      <div
        className='arcade-picker'
        role='group'
        aria-label={t('Choose a game')}
      >
        {games.map(({ id: key, name, icon: Icon }, index) => (
          <button
            type='button'
            key={key}
            aria-pressed={game === key}
            aria-controls={`${id}-${key}`}
            onClick={() => setGame(key)}
          >
            <Icon size={21} aria-hidden='true' />
            <span>{name}</span>
            <small aria-hidden='true'>0{index + 1}</small>
          </button>
        ))}
      </div>
      <div className='arcade-surface'>
        <div id={`${id}-signal`} hidden={game !== 'signal'}>
          {game === 'signal' && <SignalGamePanel />}
        </div>
        <div id={`${id}-memory`} hidden={game !== 'memory'}>
          <MemoryGame />
        </div>
        <div id={`${id}-slide`} hidden={game !== 'slide'}>
          <PuzzleGame kind='slide' />
        </div>
        <div id={`${id}-lights`} hidden={game !== 'lights'}>
          <PuzzleGame kind='lights' />
        </div>
      </div>
      <p className='arcade-footer'>
        {t('Just for fun. You can sign in or register at any time.')}
        <ArrowUpRight size={14} aria-hidden='true' />
      </p>
    </section>
  )
}

function MemoryGame() {
  const { t } = useTranslation()
  const [cards, setCards] = useState(() =>
    shuffle([0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5])
  )
  const [open, setOpen] = useState<number[]>([])
  const [matched, setMatched] = useState<number[]>([])
  const [moves, setMoves] = useState(0)
  const symbols = ['✦', '◈', '○', '△', '□', '⌁']
  useEffect(() => {
    if (open.length !== 2) return
    const timer = window.setTimeout(() => {
      if (cards[open[0]] === cards[open[1]]) {
        setMatched((previous) => [...previous, ...open])
      }
      setOpen([])
    }, 650)
    return () => window.clearTimeout(timer)
  }, [open, cards])
  const won = matched.length === cards.length
  return (
    <div className='arcade-game'>
      <div className='arcade-game-heading'>
        <h2>{t('Memory pairs')}</h2>
        <p>{t('Turn over two cards and find the matching symbols.')}</p>
      </div>
      <div className='arcade-stats'>
        <span>
          {t('Pairs found: {{count}} / 6', { count: matched.length / 2 })}
        </span>
        <span>
          {t('Moves')} <strong>{moves}</strong>
        </span>
      </div>
      <div className='arcade-board arcade-memory'>
        {cards.map((card, index) => {
          const visible = open.includes(index) || matched.includes(index)
          return (
            <button
              key={index}
              type='button'
              data-revealed={visible}
              data-matched={matched.includes(index)}
              aria-label={
                visible
                  ? t('Card {{position}}: symbol {{symbol}}', {
                      position: index + 1,
                      symbol: card + 1,
                    })
                  : t('Reveal card {{position}}', { position: index + 1 })
              }
              aria-disabled={visible || open.length === 2}
              onClick={() => {
                if (visible || open.length === 2) return
                setOpen([...open, index])
                if (open.length === 1) setMoves(moves + 1)
              }}
            >
              <span aria-hidden='true'>
                {visible ? symbols[card] : <Sparkles size={19} />}
              </span>
              {matched.includes(index) && (
                <Check
                  size={12}
                  className='arcade-match-check'
                  aria-hidden='true'
                />
              )}
            </button>
          )
        })}
      </div>
      <div className='arcade-result' role='status'>
        {won ? t('Nicely done. Play another round?') : t('Find all six pairs.')}
      </div>
      <Button
        variant='outline'
        onClick={() => {
          setCards(shuffle(cards))
          setOpen([])
          setMatched([])
          setMoves(0)
        }}
      >
        <RotateCcw size={15} />
        {t('New game')}
      </Button>
    </div>
  )
}

function PuzzleGame({ kind }: { kind: 'slide' | 'lights' }) {
  const { t } = useTranslation()
  const [tiles, setTiles] = useState(newSlide)
  const [lights, setLights] = useState(newLights)
  const [moves, setMoves] = useState(0)
  const slide = kind === 'slide'
  const won = slide ? solvedSlide(tiles) : !lights.some(Boolean)
  return (
    <div className='arcade-game'>
      <div className='arcade-game-heading'>
        <h2>{slide ? t('Sliding numbers') : t('Lights out')}</h2>
        <p>
          {slide
            ? t('Slide tiles into the empty space. Put 1 to 8 in order.')
            : t(
                'Tap a light to toggle it and its neighbors. Turn them all off.'
              )}
        </p>
      </div>
      <div className='arcade-stats'>
        <span>{slide ? '3 × 3' : '4 × 4'}</span>
        <span>
          {t('Moves')} <strong>{moves}</strong>
        </span>
      </div>
      <div
        className={`arcade-board ${slide ? 'arcade-slide' : 'arcade-lights'}`}
      >
        {slide
          ? tiles.map((tile, index) => (
              <button
                type='button'
                key={index}
                data-empty={tile === 0}
                disabled={tile === 0}
                aria-label={
                  tile
                    ? t('Move tile {{number}}', { number: tile })
                    : t('Empty space')
                }
                aria-disabled={won || slideTile(tiles, index) === tiles}
                onClick={() => {
                  if (won) return
                  const next = slideTile(tiles, index)
                  if (next !== tiles) {
                    setTiles(next)
                    setMoves(moves + 1)
                  }
                }}
              >
                {tile || ''}
              </button>
            ))
          : lights.map((on, index) => (
              <button
                type='button'
                key={index}
                aria-label={t('Light {{number}}', { number: index + 1 })}
                aria-pressed={on}
                aria-disabled={won}
                onClick={() => {
                  if (!won) {
                    setLights(toggleLights(lights, index))
                    setMoves(moves + 1)
                  }
                }}
              >
                <Lightbulb size={24} aria-hidden='true' />
              </button>
            ))}
      </div>
      <div className='arcade-result' role='status'>
        {won
          ? t('Nicely done. Play another round?')
          : slide
            ? t('One space, many possibilities.')
            : t('Every move changes the pattern.')}
      </div>
      <Button
        variant='outline'
        onClick={() => {
          if (slide) setTiles(newSlide())
          else setLights(newLights())
          setMoves(0)
        }}
      >
        <RotateCcw size={15} />
        {t('New game')}
      </Button>
    </div>
  )
}
