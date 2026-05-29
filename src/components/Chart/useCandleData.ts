import { useEffect, useMemo, useState } from 'react'

import { getOhlcv } from '../../commands/data'
import type { Candle } from '../../types/candle'

interface CandleDataState {
  candles: Candle[]
  loading: boolean
  error: string | null
}

const DAY_MS = 24 * 60 * 60 * 1000

export function useCandleData(symbol: string, timeframe: string): CandleDataState {
  const [state, setState] = useState<CandleDataState>({
    candles: [],
    loading: true,
    error: null
  })
  const range = useMemo(() => getRangeForTimeframe(timeframe), [timeframe])

  useEffect(() => {
    let cancelled = false

    async function fetchCandles() {
      setState((current) => ({ ...current, loading: true, error: null }))

      try {
        const candles = await getOhlcv(symbol, timeframe, range.from, range.to)
        if (!cancelled) {
          setState({ candles, loading: false, error: null })
        }
      } catch (error) {
        if (!cancelled) {
          setState({
            candles: [],
            loading: false,
            error: error instanceof Error ? error.message : String(error)
          })
        }
      }
    }

    fetchCandles()

    return () => {
      cancelled = true
    }
  }, [symbol, timeframe, range.from, range.to])

  return state
}

function getRangeForTimeframe(timeframe: string) {
  const to = Date.now()
  const days = timeframe === 'D1' || timeframe === 'W1' ? 365 : 30

  return {
    from: to - days * DAY_MS,
    to
  }
}
