import type { LineData, UTCTimestamp } from 'lightweight-charts'

import type { Candle } from '../../types/candle'

export interface BandsPoint {
  time: UTCTimestamp
  upper: number
  mid: number
  lower: number
}

export interface LevelLine {
  id: string
  data: LineData<UTCTimestamp>[]
}

export function simpleMovingAverage(candles: Candle[], period: number): LineData<UTCTimestamp>[] {
  const output: LineData<UTCTimestamp>[] = []
  let sum = 0

  candles.forEach((candle, index) => {
    sum += candle.close
    if (index >= period) {
      sum -= candles[index - period].close
    }

    if (index + 1 >= period) {
      output.push({
        time: toTime(candle),
        value: sum / period
      })
    }
  })

  return output
}

export function exponentialMovingAverage(
  candles: Candle[],
  period: number
): LineData<UTCTimestamp>[] {
  if (period <= 0) {
    return []
  }

  const output: LineData<UTCTimestamp>[] = []
  const multiplier = 2 / (period + 1)
  let warmup = 0
  let previous: number | undefined

  candles.forEach((candle, index) => {
    if (index + 1 < period) {
      warmup += candle.close
      return
    }

    if (index + 1 === period) {
      warmup += candle.close
      previous = warmup / period
    } else {
      previous = (candle.close - previous!) * multiplier + previous!
    }

    output.push({
      time: toTime(candle),
      value: previous
    })
  })

  return output
}

export function bollingerBands(
  candles: Candle[],
  period: number,
  multiplier: number
): BandsPoint[] {
  const mids = exponentialMovingAverage(candles, period)
  const midByTime = new Map(mids.map((point) => [point.time, point.value]))
  const output: BandsPoint[] = []

  candles.forEach((candle, index) => {
    if (index + 1 < period) {
      return
    }

    const time = toTime(candle)
    const mid = midByTime.get(time)
    if (mid === undefined) {
      return
    }

    const start = index + 1 - period
    const variance =
      candles
        .slice(start, index + 1)
        .reduce((sum, item) => sum + Math.pow(item.close - mid, 2), 0) / period
    const stdDev = Math.sqrt(variance)

    output.push({
      time,
      upper: mid + stdDev * multiplier,
      mid,
      lower: mid - stdDev * multiplier
    })
  })

  return output
}

export function supportResistanceLevels(candles: Candle[], lookback = 90): LevelLine[] {
  const visible = candles.slice(-lookback)
  if (visible.length < 8) {
    return []
  }

  const support = Math.min(...visible.map((candle) => candle.low))
  const resistance = Math.max(...visible.map((candle) => candle.high))
  const first = toTime(visible[0])
  const last = toTime(visible[visible.length - 1])

  return [
    {
      id: 'support',
      data: [
        { time: first, value: support },
        { time: last, value: support }
      ]
    },
    {
      id: 'resistance',
      data: [
        { time: first, value: resistance },
        { time: last, value: resistance }
      ]
    }
  ]
}

function toTime(candle: Candle): UTCTimestamp {
  return Math.floor(candle.timestamp / 1000) as UTCTimestamp
}
