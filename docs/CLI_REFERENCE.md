# AlgoMLN CLI Command Structure & Reference

This document provides a complete reference for the `algomln` command-line interface, including the strategy execution engine (`algomln run`) and market data catalog commands (`algomln data`).

---

## 1. Quick Syntax Overview

When running from the repository root via Cargo:
```bash
cargo run -p algomln-cli -- <command> [options]
```
Or with a compiled binary:
```bash
algomln <command> [options]
```

Root commands:
```text
algomln run <strategy> [options]
algomln data <status|verify|update|bulk|import> [options]
algomln --help | -h
```

---

## 2. Market Data Commands (`algomln data`)

The `data` suite manages the local partitioned Parquet store and SQLite catalog (`catalog.sqlite`).

```text
algomln data <subcommand> [options]
```

### 2.1. `algomln data update`
Fetches historical candle data for specific symbols from the Dhan API and updates the local Parquet partitions and SQLite catalog.

```bash
cargo run -p algomln-cli -- data update \
  --master <path/to/api-scrip-master-detailed.csv> \
  --symbols <SYMBOL1,SYMBOL2,...> \
  [--tf <1m|1d>] \
  [--from <YYYY-MM-DD>] \
  [--to <YYYY-MM-DD>] \
  [--reference <SYMBOL>] \
  [--data <DIR>]
```

#### Arguments & Options
| Option | Required | Default | Description |
|---|---|---|---|
| `--master` | **Yes** | — | Path to detailed Dhan scrip master CSV. Updates the internal instruments catalog. |
| `--symbols` | **Yes** | — | Comma-separated list of symbols (e.g. `RELIANCE,TCS,INFY`). |
| `--tf` | No | `1m` | Candle timeframe: `1m`, `5m`, `15m`, `25m`, `60m`, `1d`. |
| `--from` | No | `to - 5 years` | Download start date (`YYYY-MM-DD`). |
| `--to` | No | Today (`Utc::now()`) | Download end date (`YYYY-MM-DD`). |
| `--reference` | No | `RELIANCE` | Symbol used as trading session & calendar anchor. |
| `--data` | No | `data` | Root directory for Parquet data and `catalog.sqlite`. |

#### "No Dates" Mode (Default 5-Year Window)
When `--from` and `--to` are omitted:
- `--to` automatically defaults to the current day.
- `--from` defaults to exactly 5 years prior to `--to`.
- The updater inspects existing catalog coverage and only requests missing chunks from the API. If already cached, it prints `already up to date`.

**Example:**
```bash
cargo run -p algomln-cli -- data update \
  --master sample-data/api-scrip-master-detailed.csv \
  --symbols RELIANCE \
  --tf 1m
```

---

### 2.2. `algomln data bulk`
Designed for downloading data across multiple symbols or the entire universe (`nse-eq`), supporting multi-worker concurrency, budget management, resume-on-failure, and dry-run planning.

```bash
cargo run -p algomln-cli -- data bulk \
  --master <path/to/api-scrip-master-detailed.csv> \
  --from <YYYY-MM-DD> \
  [--to <YYYY-MM-DD>] \
  [--universe nse-eq] \
  [--tf <1d,1m>] \
  [--symbols <path/to/symbols.txt>] \
  [--budget <N>] \
  [--concurrency <N>] \
  [--reference <SYMBOL>] \
  [--dry-run] \
  [--data <DIR>]
```

#### Arguments & Options
| Option | Required | Default | Description |
|---|---|---|---|
| `--master` | **Yes** | — | Path to detailed Dhan scrip master CSV. |
| `--from` | **Yes** | — | Start date (`YYYY-MM-DD`). |
| `--to` | No | Last completed session | End date (`YYYY-MM-DD`). Excludes weekends and in-progress sessions. |
| `--universe` | No | `nse-eq` | Scrip universe filter (`nse-eq` only). |
| `--tf` | No | `1d,1m` | Comma-separated list of timeframes. Daily (`1d`) is always downloaded first across all symbols to establish baselines, followed by `1m`. |
| `--symbols` | No | All universe | Path to a text file with newline-separated trading symbols to prioritize first. |
| `--budget` | No | `90000` | Daily request cap for IST day (Dhan hard cap is 100,000). |
| `--concurrency`| No | `4` | Worker threads across symbols. Request start spacing is strictly enforced at ≥250 ms by the shared rate limiter. |
| `--dry-run` | No | Flag | Runs the planner without sending HTTP requests. Prints symbol count, planned requests per timeframe, estimated duration at 4 req/s, and budget fit. |
| `--reference` | No | `RELIANCE` | Calendar anchor symbol. |
| `--data` | No | `data` | Storage directory. |

#### Dry Run Example
```bash
cargo run -p algomln-cli -- data bulk \
  --universe nse-eq \
  --tf 1d,1m \
  --from 2021-01-01 \
  --dry-run \
  --master sample-data/api-scrip-master-detailed.csv
```

#### Live Bulk Download Example
```bash
cargo run -p algomln-cli -- data bulk \
  --universe nse-eq \
  --tf 1d,1m \
  --from 2021-01-01 \
  --concurrency 4 \
  --master sample-data/api-scrip-master-detailed.csv
```

> [!TIP]
> **Interruption & Resumption:** `Ctrl-C` is intercepted safely. In-flight requests finish, SQLite catalog updates and Parquet writes commit, and progress is preserved. Running the same command again will seamlessly resume where it left off because the planner derives work from catalog gaps.

---

### 2.3. `algomln data status`
Displays the current state of the local data catalog: instrument counts, stored series, bar counts, and date coverage ranges.

```bash
cargo run -p algomln-cli -- data status [--data <DIR>]
```

**Output Includes:**
- Path to `catalog.sqlite`
- Total equities and F&O-eligible count
- Stored series list (`<symbol> <tf>: <bars> bars`)
- Date coverage intervals (`<from>..<to> <kind>`)
- Data quality flags (e.g. gaps, suspicious sessions)

---

### 2.4. `algomln data verify`
Reconciles on-disk Parquet files against the SQLite catalog metadata to fix inconsistencies or register unindexed files.

```bash
cargo run -p algomln-cli -- data verify [--data <DIR>]
```

---

### 2.5. `algomln data import`
Imports existing CSV or Parquet files into the catalog and partitioned store.

```bash
cargo run -p algomln-cli -- data import \
  --symbol <SYMBOL> \
  --file <path/to/data.csv|data.parquet> \
  [--tf <1m|1d>] \
  [--csv-tz <TIMEZONE>] \
  [--data <DIR>]
```

#### CSV Column Requirements:
CSVs must contain headers: `ts` (Unix epoch seconds or ISO timestamp), `open`, `high`, `low`, `close`, `volume`.

---

## 3. Backtest & Simulation (`algomln run`)

Executes a compiled strategy written in AlgoMLN DSL against market data.

```bash
cargo run -p algomln-cli -- run <strategy.algomln> [options]
```

### 3.1. General Simulation Options
| Option | Default | Description |
|---|---|---|
| `<strategy.algomln>` | **Required** | Path to the strategy definition file. |
| `--data <DIR>` | `data` | Root path to Parquet partitions and `catalog.sqlite`. |
| `--tf <TF>` | `1m` | Timeframe to evaluate (`1m`, `5m`, `15m`, `25m`, `60m`, `1d`). |
| `--symbols <A,B>` | From strategy | Comma-separated symbols override. If omitted, uses the symbols specified in the strategy's `TRADE_IN` block. |
| `--from <YYYY-MM-DD>` | Auto | Start date for backtest (IST). |
| `--to <YYYY-MM-DD>` | Auto | End date for backtest (inclusive up to 23:59:59 IST). |
| `--cash <AMOUNT>` | `100000.0` | Initial capital. |
| `--warmup-bars <N>` | Auto | Number of leading bars fed to warm up indicators before trade rules evaluate. |
| `--latency <N>` | `1` | Order fill latency in candles (minimum `1`). |
| `--slippage-bps <N>` | `0.0` | Slippage applied to fills in basis points (`1 bps = 0.01%`). Range: `[0, 10000)`. |
| `--fee-bps <N>` | `0.0` | Transaction fee model in basis points. |
| `--partial-fills <PCT>`| None | Percentage of requested quantity filled per order in `(0, 100]`. |
| `--gap-policy <POLICY>`| `none` | Policy for missing intraday candles: `none` or `fill` (flat forward-fill within session). |
| `--allow-gaps` | Flag | Permit missing ranges in data without halting execution. |
| `--debug-invariants` | Flag | Enables internal simulation invariant checks during the run. |
| `--out <DIR>` | `run-output` | Directory where backtest outputs (metrics, trades, equity curve) are written. |

### 3.2. Automatic Pre-Run Data Update (`--update`)
Pass `--update` to tell `algomln run` to query catalog coverage for the target symbols and date range, download any missing gaps from Dhan API, and immediately continue into the backtest.

```bash
cargo run -p algomln-cli -- run examples/momentum.algomln \
  --update \
  --master sample-data/api-scrip-master-detailed.csv \
  [--reference RELIANCE]
```

### 3.3. Legacy CSV Source Options
Used when streaming from raw non-catalog CSV files:
- `--csv-tz <TZ>`: Timezone for CSV timestamps (e.g. `Asia/Kolkata`).
- `--skip-bad`: Ignores malformed bars instead of terminating.

---

## 4. Environment Variables & Authentication

Connecting to the Dhan API requires access credentials. These can be set via environment variables or stored securely in your OS keychain.

| Variable | Description |
|---|---|
| `DHAN_ACCESS_TOKEN` | Dhan API Bearer token. |
| `DHAN_CLIENT_ID` | Dhan 10-digit Client ID. |
| `DHAN_API_BASE` | API base URL (default: `https://api.dhan.co/v2`). |

### Keyring Alternative
AlgoMLN also checks the OS keychain for credentials under:
- **Service:** `algomln`
- **Account:** `dhan-access-token`
