import { invoke } from '@tauri-apps/api/core'

import type { Candle, Quote } from '../types/candle'

export const getOhlcv = (
  symbol: string,
  timeframe: string,
  from: number,
  to: number
): Promise<Candle[]> => {
  if (!isTauri()) {
    return Promise.resolve(demoCandles(from, to))
  }

  return invoke<Candle[]>('get_ohlcv', { symbol, timeframe, from, to })
}

export const getQuote = (symbol: string): Promise<Quote> => {
  if (!isTauri()) {
    return Promise.resolve({
      symbol,
      ltp: 0,
      open: 0,
      high: 0,
      low: 0,
      close: 0,
      bid: 0,
      ask: 0,
      volume: 0
    })
  }

  return invoke<Quote>('get_quote', { symbol })
}

function isTauri() {
  return '__TAURI_INTERNALS__' in window
}

function demoCandles(from: number, to: number): Candle[] {
  const count = 120
  const step = Math.max(Math.floor((to - from) / count), 60_000)
  let close = 1350

  return Array.from({ length: count }, (_, index) => {
    const open = close
    const wave = Math.sin(index / 5) * 8
    close = Math.max(1, open + wave + (index % 7) - 3)
    const high = Math.max(open, close) + 6 + (index % 4)
    const low = Math.min(open, close) - 6 - (index % 3)

    return {
      timestamp: from + index * step,
      open,
      high,
      low,
      close,
      volume: 100_000 + index * 1_000
    }
  })
}
