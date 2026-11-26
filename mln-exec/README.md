# mln-exec

Whole-array reference oracle interpreter (and upcoming Phase 4 incremental executor) for **LangMelogne** (`IrGraph`).

## Public API

- `eval_oracle(graph: &IrGraph, inputs: &SeriesInputs) -> Result<OracleOutput, ExecError>`
- `eval_source_oracle(src: &str, inputs: &SeriesInputs) -> Result<(IrGraph, OracleOutput), ExecError>`
- `SeriesInputs`, `OracleOutput`, `ValueSeries`, `ExecError`

## Example

```rust
use mln_exec::{eval_source_oracle, SeriesInputs};

let bars = SeriesInputs {
    open: vec![10.0, 11.0, 12.0],
    high: vec![11.0, 12.0, 13.0],
    low: vec![9.5, 10.5, 11.5],
    close: vec![10.5, 10.0, 12.5],
    volume: vec![100.0, 120.0, 140.0],
};
let src = "entry = close > open\nexit = close < open";
let (_, out) = eval_source_oracle(src, &bars).unwrap();
assert_eq!(out.entry_signals, vec![1, 0, 1]);
assert_eq!(out.exit_signals, vec![0, 1, 0]);
```
