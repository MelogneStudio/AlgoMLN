import { useEffect, useRef, type RefObject } from 'react'
import {
  CandlestickSeries,
  ColorType,
  createChart,
  CrosshairMode,
  LineSeries,
  type CandlestickData,
  type IChartApi,
  type ISeriesApi,
  type LineData,
  type UTCTimestamp
} from 'lightweight-charts'

interface LineConfig {
  color: string
  lineWidth?: 1 | 2 | 3 | 4
  lineStyle?: 0 | 1 | 2 | 3 | 4
}

export interface ChartController {
  setCandles(data: CandlestickData<UTCTimestamp>[]): void
  setLineSeries(id: string, data: LineData<UTCTimestamp>[], config: LineConfig): void
  removeLineSeries(id: string): void
  fitContent(): void
}

export function useChart(containerRef: RefObject<HTMLDivElement | null>) {
  const chartRef = useRef<IChartApi | null>(null)
  const candleSeriesRef = useRef<ISeriesApi<'Candlestick'> | null>(null)
  const lineSeriesRef = useRef(new Map<string, ISeriesApi<'Line'>>())

  useEffect(() => {
    const container = containerRef.current
    if (!container) {
      return
    }

    const chart = createChart(container, {
      autoSize: true,
      layout: {
        background: { type: ColorType.Solid, color: '#0f1318' },
        textColor: '#aab3c2'
      },
      grid: {
        vertLines: { color: '#1c232d' },
        horzLines: { color: '#1c232d' }
      },
      crosshair: {
        mode: CrosshairMode.Normal
      },
      rightPriceScale: {
        borderColor: '#252b33'
      },
      timeScale: {
        borderColor: '#252b33',
        timeVisible: true
      }
    })
    const candleSeries = chart.addSeries(CandlestickSeries, {
      upColor: '#22ab94',
      downColor: '#f23645',
      borderUpColor: '#22ab94',
      borderDownColor: '#f23645',
      wickUpColor: '#22ab94',
      wickDownColor: '#f23645'
    })

    chartRef.current = chart
    candleSeriesRef.current = candleSeries

    return () => {
      lineSeriesRef.current.clear()
      chart.remove()
      chartRef.current = null
      candleSeriesRef.current = null
    }
  }, [containerRef])

  return {
    setCandles(data) {
      candleSeriesRef.current?.setData(data)
    },
    setLineSeries(id, data, config) {
      const chart = chartRef.current
      if (!chart) {
        return
      }

      let series = lineSeriesRef.current.get(id)
      if (!series) {
        series = chart.addSeries(LineSeries, {
          color: config.color,
          lineWidth: config.lineWidth ?? 2,
          lineStyle: config.lineStyle ?? 0,
          priceLineVisible: false,
          lastValueVisible: false
        })
        lineSeriesRef.current.set(id, series)
      }

      series.applyOptions({
        color: config.color,
        lineWidth: config.lineWidth ?? 2,
        lineStyle: config.lineStyle ?? 0
      })
      series.setData(data)
    },
    removeLineSeries(id) {
      const chart = chartRef.current
      const series = lineSeriesRef.current.get(id)
      if (!chart || !series) {
        return
      }

      chart.removeSeries(series)
      lineSeriesRef.current.delete(id)
    },
    fitContent() {
      chartRef.current?.timeScale().fitContent()
    }
  } satisfies ChartController
}
