/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { WeeklyClient, WeeklyInspection } from './api'

export type WeeklyDetailState = {
  phase:
    | 'idle'
    | 'loading'
    | 'ready'
    | 'claiming'
    | 'unknown'
    | 'error'
    | 'cleared'
  inspection: WeeklyInspection | null
}

/** Private browser memory only. The server remains the claim authority. */
export class WeeklyDetailSession {
  private state: WeeklyDetailState = { phase: 'idle', inspection: null }
  private listeners = new Set<() => void>()
  private generation = 0
  private request: AbortController | null = null
  constructor(
    readonly id: string,
    private client: WeeklyClient,
    private current: () => boolean
  ) {}
  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  private setState(state: WeeklyDetailState) {
    this.state = state
    for (const listener of this.listeners) {
      listener()
    }
  }
  clear = () => {
    this.generation++
    this.request?.abort()
    this.request = null
    this.setState({ phase: 'cleared', inspection: null })
  }
  query = async () => {
    if (this.request || !this.current()) {
      return
    }
    const generation = ++this.generation
    const request = new AbortController()
    this.request = request
    const recovering = this.state.phase === 'unknown'
    this.setState({ phase: 'loading', inspection: null })
    try {
      const inspection = await this.client.inspect(this.id, request.signal)
      if (this.current() && generation === this.generation) {
        this.setState({ phase: 'ready', inspection })
      }
    } catch {
      if (this.current() && generation === this.generation) {
        this.setState({
          phase: recovering ? 'unknown' : 'error',
          inspection: null,
        })
      }
    } finally {
      if (generation === this.generation) {
        this.request = null
      }
    }
  }
  claim = async () => {
    const { inspection, phase } = this.state
    if (
      this.request ||
      !this.current() ||
      phase !== 'ready' ||
      !inspection?.claimable ||
      !inspection.csrf
    ) {
      return
    }
    const generation = ++this.generation
    const request = new AbortController()
    this.request = request
    const csrf = inspection.csrf
    this.setState({
      phase: 'claiming',
      inspection: {
        ok: true,
        decision: inspection.decision,
        claimable: false,
      },
    })
    try {
      const claimed = await this.client.claim(
        inspection.decision,
        csrf,
        request.signal
      )
      if (this.current() && generation === this.generation) {
        this.setState({
          phase: 'ready',
          inspection: {
            ok: true,
            decision: claimed.decision,
            claimable: false,
          },
        })
      }
    } catch {
      if (this.current() && generation === this.generation) {
        this.setState({
          phase: 'unknown',
          inspection: {
            ok: true,
            decision: inspection.decision,
            claimable: false,
          },
        })
      }
    } finally {
      if (generation === this.generation) {
        this.request = null
      }
    }
  }
}
