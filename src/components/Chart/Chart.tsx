import { useEffect, useMemo, useRef } from 'react'

import type { IndicatorSettings } from '../../types/indicator'
import { toLWCandle } from '../../types/candle'
import {
  bollingerBands,
  exponentialMovingAverage,
  simpleMovingAverage,
  supportResistanceLevels
} from './indicators'
import { useCandleData } from './useCandleData'
import { useChart } from './useChart'

interface ChartProps {
  symbol: string
  timeframe: string
  indicators: IndicatorSettings
}

export function Chart({ symbol, timeframe, indicators }: ChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const chart = useChart(containerRef)
  const { candles, loading, error } = useCandleData(symbol, timeframe)
  const lwCandles = useMemo(() => candles.map(toLWCandle), [candles])

  useEffect(() => {
    chart.setCandles(lwCandles)
    chart.fitContent()
  }, [chart, lwCandles])

  useEffect(() => {
    if (indicators.ma20) {
      chart.setLineSeries('ma20', simpleMovingAverage(candles, 20), {
        color: '#f2c94c',
        lineWidth: 2
      })
    } else {
      chart.removeLineSeries('ma20')
    }

    if (indicators.ema20) {
      chart.setLineSeries('ema20', exponentialMovingAverage(candles, 20), {
        color: '#56ccf2',
        lineWidth: 2
      })
    } else {
      chart.removeLineSeries('ema20')
    }

    if (indicators.bb20) {
      const bands = bollingerBands(candles, 20, 2)
      chart.setLineSeries(
        'bb20-upper',
        bands.map((point) => ({ time: point.time, value: point.upper })),
        { color: '#9b7cf6', lineWidth: 1 }
      )
      chart.setLineSeries(
        'bb20-mid',
        bands.map((point) => ({ time: point.time, value: point.mid })),
        { color: '#6f7683', lineWidth: 1 }
      )
      chart.setLineSeries(
        'bb20-lower',
        bands.map((point) => ({ time: point.time, value: point.lower })),
        { color: '#9b7cf6', lineWidth: 1 }
      )
    } else {
      chart.removeLineSeries('bb20-upper')
      chart.removeLineSeries('bb20-mid')
      chart.removeLineSeries('bb20-lower')
    }

    if (indicators.supportResistance) {
      for (const level of supportResistanceLevels(candles)) {
        chart.setLineSeries(`sr-${level.id}`, level.data, {
          color: level.id === 'support' ? '#2f9e44' : '#d9480f',
          lineWidth: 1,
          lineStyle: 2
        })
      }
    } else {
      chart.removeLineSeries('sr-support')
      chart.removeLineSeries('sr-resistance')
    }
  }, [candles, chart, indicators])

  return (
    <div className="chart-shell">
      <div ref={containerRef} className="chart-container" />
      <div className={error ? 'status-bar status-error' : 'status-bar'}>
        {error
          ? error
          : loading
            ? 'Loading candles'
            : `${candles.length} candles • ${symbol} • ${timeframe}`}
      </div>
    </div>
  )
}
