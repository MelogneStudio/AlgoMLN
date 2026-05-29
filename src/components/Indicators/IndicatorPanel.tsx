import type { IndicatorSettings } from '../../types/indicator'

interface IndicatorPanelProps {
  value: IndicatorSettings
  onChange(value: IndicatorSettings): void
}

export function IndicatorPanel({ value, onChange }: IndicatorPanelProps) {
  return (
    <div className="indicator-panel">
      <span>Overlays</span>
      <Toggle
        label="MA 20"
        checked={value.ma20}
        onChange={(checked) => onChange({ ...value, ma20: checked })}
      />
      <Toggle
        label="EMA 20"
        checked={value.ema20}
        onChange={(checked) => onChange({ ...value, ema20: checked })}
      />
      <Toggle
        label="BB 20"
        checked={value.bb20}
        onChange={(checked) => onChange({ ...value, bb20: checked })}
      />
      <Toggle
        label="S/R"
        checked={value.supportResistance}
        onChange={(checked) => onChange({ ...value, supportResistance: checked })}
      />
    </div>
  )
}

interface ToggleProps {
  label: string
  checked: boolean
  onChange(checked: boolean): void
}

function Toggle({ label, checked, onChange }: ToggleProps) {
  return (
    <label className="toggle">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  )
}
