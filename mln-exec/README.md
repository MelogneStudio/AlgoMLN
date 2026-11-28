# mln-exec

Reference oracle interpreter and zero-allocation incremental streaming executor for **LangMelogne** (`IrGraph`).

## Public API

- **Incremental Executor (Phase 4)**:
  - `Executor::new(ir: &IrGraph) -> Result<Executor, ExecError>`
  - `Executor::step(&mut self, bar: Bar) -> Result<BarOutput, ExecError>`
  - `Executor::run_chunk(&mut self, bars: &[Bar]) -> Result<Vec<BarOutput>, ExecError>`
  - `Executor::run_chunk_into(&mut self, bars: &[Bar], out: &mut Vec<BarOutput>) -> Result<(), ExecError>`
  - `Executor::snapshot(&self) -> ExecutorSnapshot` / `Executor::restore(&mut self, snap: &ExecutorSnapshot) -> Result<(), ExecError>`
  - `Bar`, `BarOutput`, `NodeVal`, `ExecutorSnapshot`
- **Whole-Array Oracle (Phase 3)**:
  - `eval_oracle(graph: &IrGraph, inputs: &SeriesInputs) -> Result<OracleOutput, ExecError>`
  - `eval_source_oracle(src: &str, inputs: &SeriesInputs) -> Result<(IrGraph, OracleOutput), ExecError>`
  - `SeriesInputs`, `OracleOutput`, `ValueSeries`, `ExecError`

## Incremental Example

```rust
use mln_exec::{Bar, Executor};
use mln_ir::lower_default;
use mln_lang::parse;

let src = "s = sma(close, 3)\nentry = s > open\nexit = close < open";
let parsed = parse(src);
assert!(parsed.diags.is_empty());
let ir = lower_default(&parsed.program).unwrap();

let mut exec = Executor::new(&ir).unwrap();
let bars = [
    Bar::new(1_700_000_000, 10.0, 11.0, 9.5, 10.5, 100.0),
    Bar::new(1_700_000_060, 11.0, 12.0, 10.5, 11.5, 120.0),
    Bar::new(1_700_000_120, 12.0, 13.0, 11.5, 12.5, 140.0),
];
let mut out = Vec::with_capacity(bars.len());
exec.run_chunk_into(&bars, &mut out).unwrap();
assert_eq!(out.len(), 3);
assert_eq!(out[2].entry_signal, 1);

let snap = exec.snapshot();
let mut resumed = Executor::new(&ir).unwrap();
resumed.restore(&snap).unwrap();
```

## Benchmark (`T4-07`)

- Criterion 1M-bar benchmark: `cargo bench -p mln-exec --bench exec_1m`
- Local 15% regression gate (release): `cargo test -p mln-exec --release --test t4_07_regression`
