interface SymbolSearchProps {
  value: string
  onChange(value: string): void
}

export function SymbolSearch({ value, onChange }: SymbolSearchProps) {
  return (
    <label className="field">
      <span>Symbol</span>
      <input
        className="symbol-input"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        spellCheck={false}
      />
    </label>
  )
}
