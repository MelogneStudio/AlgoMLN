<img src=".github/algomln-logo.svg">

AlgoMLN is a fast, local platform for algorithmic trading.

## Historical simulation

Run a strategy against local OHLCV data with the `run` command. A timeframe is
required because it is a run setting, not part of the strategy language.

```powershell
cargo run --release -- run .\samples\trend_momentum.algomln `
  --data .\market-data --tf 1m --cash 100000 --out .\run-output
```

The data directory contains one `<SYMBOL>.csv` or `<SYMBOL>.parquet` file per
symbol. CSV files have `ts,open,high,low,close,volume` columns. `ts` accepts
epoch seconds or RFC3339 timestamps with an offset. For naive local timestamps,
pass a timezone such as `--csv-tz Asia/Kolkata`. Parquet files use `ts` as i64
and the five OHLCV fields as f64.

Optional run settings include `--symbols A,B`, `--from YYYY-MM-DD`,
`--to YYYY-MM-DD`, `--warmup-bars N`, `--latency N`, `--slippage-bps N`,
`--fee-bps N`, `--partial-fills PCT`, `--gap-policy none|fill`, `--skip-bad`,
and `--debug-invariants`. The local source and simulator do not use network or
broker APIs.

The output directory contains `fills.csv`, `trades.csv`, `equity.csv`,
`metrics.json`, `manifest.json`, `open_positions.json`, and `events.json`.

Run the single-symbol and three-symbol synthetic throughput and peak-allocation
benchmarks with `cargo bench --bench sim_pipeline`.
