const timeframes = ['M1', 'M5', 'M15', 'M25', 'M60', 'D1', 'W1']

interface TimeframeSelectorProps {
  value: string
  onChange(value: string): void
}

export function TimeframeSelector({ value, onChange }: TimeframeSelectorProps) {
  return (
    <label className="field">
      <span>Timeframe</span>
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {timeframes.map((timeframe) => (
          <option key={timeframe} value={timeframe}>
            {timeframe}
          </option>
        ))}
      </select>
    </label>
  )
}
