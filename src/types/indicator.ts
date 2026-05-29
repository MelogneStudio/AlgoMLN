import type { LineData, UTCTimestamp } from 'lightweight-charts'

export type IndicatorSeries = LineData<UTCTimestamp>

export interface IndicatorSettings {
  ma20: boolean
  ema20: boolean
  bb20: boolean
  supportResistance: boolean
}
