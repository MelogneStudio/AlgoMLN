import { useState } from 'react'

import { Chart } from './components/Chart/Chart'
import { IndicatorPanel } from './components/Indicators/IndicatorPanel'
import { SymbolSearch } from './components/Toolbar/SymbolSearch'
import { TimeframeSelector } from './components/Toolbar/TimeframeSelector'
import type { IndicatorSettings } from './types/indicator'

export function App() {
  const [symbol, setSymbol] = useState('2885|NSE_EQ|EQUITY')
  const [timeframe, setTimeframe] = useState('D1')
  const [indicators, setIndicators] = useState<IndicatorSettings>({
    ma20: true,
    ema20: true,
    bb20: false,
    supportResistance: true
  })

  return (
    <main className="app">
      <section className="workspace">
        <div className="topbar">
          <SymbolSearch value={symbol} onChange={setSymbol} />
          <TimeframeSelector value={timeframe} onChange={setTimeframe} />
          <IndicatorPanel value={indicators} onChange={setIndicators} />
        </div>
        <Chart symbol={symbol} timeframe={timeframe} indicators={indicators} />
      </section>
    </main>
  )
}
