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

Simulation ledger money is stored in integer paise. Candle and broker prices
remain rupee-denominated `f64` at the market-data boundary, then convert to
paise for ledger accounting, sizing, and risk checks. Fees use round-half-to-even
to the nearest paise; ties go to the even paise. Result artifacts include exact
`*_paise` fields alongside readable rupee values. The broker stores fee rates as
integer hundredths of a basis point for fee multiplication and rounds simulated
fills adversely to the configured tick size (5 paise by default).

## Dhan NSE equities data layer

The `data` catalog stores native Dhan timeframes independently. It does not resample bars. Intraday history is split/bonus adjusted by Dhan. Daily bars use the exchange official close from the closing auction; their high/low/close can differ from the intraday series. Simulation equity is marked using the last candle close of the selected run timeframe.

Use `algomln data update --master <scrip-master.csv> --symbols RELIANCE,TCS --tf 1m --from YYYY-MM-DD --to YYYY-MM-DD` to update NSE cash equities. Set `DHAN_ACCESS_TOKEN` in the environment. `algomln data status` shows persisted coverage and `algomln data verify` reconciles catalog entries with Parquet files. `algomln run` uses the catalog by default; network access occurs only with `--update`.

The Dhan data API documents 5 requests/second and 100,000 requests/day. AlgoMLN spaces starts at 250 ms and applies a 90,000 request soft daily cap; after DH-904 it halves the request rate.
