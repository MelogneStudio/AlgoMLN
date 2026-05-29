import type { CandlestickData, UTCTimestamp } from 'lightweight-charts'

export interface Candle {
  timestamp: number
  open: number
  high: number
  low: number
  close: number
  volume: number
}

export interface Quote {
  symbol: string
  ltp: number
  open: number
  high: number
  low: number
  close: number
  bid: number
  ask: number
  volume: number
}

export type LWCandle = CandlestickData<UTCTimestamp>

export function toLWCandle(candle: Candle): LWCandle {
  return {
    time: Math.floor(candle.timestamp / 1000) as UTCTimestamp,
    open: candle.open,
    high: candle.high,
    low: candle.low,
    close: candle.close
  }
}
