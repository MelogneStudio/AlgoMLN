# AlgoMLN — Remaining Work (handoff)

Handoff note for a fresh session. As of 2026-10-31. Everything below is **open**;
the "Done" section is context so you don't redo it.

## Repo / workflow facts
- Rust workspace. Crates: root lib (`algomln`: DSL, indicators, engine, trader, sim),
  `data` (`algomln-data`: catalog + bulk download), `dhan-api`, `cli` (`algomln-cli`).
- Branch: local `algomln2` tracks remote **`AlgoMLN2`** (note the case). Push with
  `git push origin algomln2:AlgoMLN2`. `git pull` fails on the case mismatch; use
  `git fetch` + `git merge --ff-only origin/AlgoMLN2`.
- Commit style: conventional commits (`feat(scope): ...`, `fix(...)`, `test(...)`,
  `docs(...)`), short lowercase subject, **no `Co-Authored-By` trailer** (repo convention).
- If working through a subagent that cannot compile: do NOT run cargo there; write
  carefully, commit locally, let the human compile and push. The human verifies with
  `cargo test --workspace` / `cargo build -p algomln-cli` on their PC.
- Spec for the data layer bulk download: `raw/3d5b.txt`. Concurrency design: `raw/3d5b-concurrency-design.md`.

## Done (context — do not redo)
- Data bulk download "Step 5b" (`raw/3d5b.txt`): `failures` table, `data bulk` CLI
  command, budget hard-stop, `--dry-run`, resume via coverage-derived planning,
  progress line + summary, §11 mock-Dhan acceptance tests, §7 Ctrl-C handler. All green.
- `catalog_source` benchmark fixed + made tractable; measures ~924k candles/sec
  (vs ~52k in AlgoMLN 1.x). Loads ~46M candles/iter (~2 GiB), pinned to 10 samples.
- Correctness fixes already landed: VWAP IST daily reset (was ms divisor),
  `Candle::is_valid` full `low <= {open,close} <= high`, parser current-depth nesting,
  stop-loss/take-profit in integer paise, overflow-safe sizing casts, duplicate-candle
  guard in the sim frame loop.
- Audit items intentionally **dismissed after re-analysis** (not bugs): M1 (RSI/rel_vol
  warmup +1 — feeding `period` prior bars already makes them ready on the first acting
  bar); M3 (Observe-mode marks — no position can exist without Act mode placing an order);
  H4/L4 (EMA/ATR non-finite — `is_valid` already rejects non-finite fields).

---

## A. Step 5b leftover — concurrency semaphore (§6)
- **What:** up to 4 in-flight Dhan requests, with request *starts* still ≥250ms apart
  (the shared `Limiter` enforces spacing; the semaphore must not override it). Improves
  wall-clock when request latency > 250ms (~2× for the 1m full-universe run).
- **Status:** deferred with a full design in `raw/3d5b-concurrency-design.md`. Serial path
  is intact and correct.
- **Why it's non-trivial:** `Updater`/`Catalog` hold a single non-`Sync` SQLite connection,
  and fetch/write/overlap-detection are interleaved. The design splits a bounded
  concurrent *fetch* from a serial *writer*; keep serial: budget increment, per-series
  overlap read-modify-write, per-symbol commit boundary (needed for resume + Ctrl-C).
- **Start here:** read `raw/3d5b-concurrency-design.md`, then `data/src/lib.rs`
  (`BulkDownloader`, `Updater::update`, `Catalog`, `Store::write_partitions`) and
  `dhan-api/src/lib.rs` (`DhanClient`, `Limiter`).

## B. Audit bugs still open
File:line references are approximate (pre-existing commits may have shifted them a little).

### H1 — two divergent money rounders (High)
- `src/trader.rs:~12` `rupees_to_paise` rounds in `f64` (ties-to-even via floor+parity),
  while `round_half_even_ratio` / `sim.rs:~755` round in `i128`. For negative or exact-half
  inputs, and for large values (f64 52-bit mantissa), the two paths can disagree by 1 paise.
- **Fix:** route all rupee→paise conversion through the integer path (`decimal_scaled`);
  make `rupees_to_paise` a thin wrapper or remove the float path. Add negative + exact-tie
  tests. **Small, self-contained, money-correctness — good first item.**

### M4 — gap-fill is a no-op on daily data (Medium)
- `src/sim.rs:~303,~306`: `&& self.step < 86400` disables gap filling and `missing_candles`
  for `Timeframe::D1`, so `GapPolicy::FillFlatInSession` does nothing on D1. `in_ist_session`
  also limits intraday counting to 09:15–15:30, so cross-session/overnight gaps aren't counted.
- **Fix:** handle (or explicitly document) D1 gap policy and overnight gap reporting.

### M5 — per-symbol equity is O(n²) + approximation (Medium)
- `src/sim.rs:~1392-1411, ~1093-1120`: `symbol_pnl` replays the whole fills list every frame
  per symbol (quadratic on long runs); per-symbol equity uses a static even cash split that
  won't sum to portfolio equity when cash is unevenly deployed.
- **Fix:** maintain incremental per-symbol P&L state.

### Cosmetic / latent (Low)
- **M2** `src/trader.rs:~271,~295,~303,~493`: `daily_loss_locked` is assigned but never read
  (the gate works via recompute). Remove it or wire it up.
- **L2** `src/lexer/mod.rs:~199-232`: time literals like `25:99` lex as valid `Time`;
  only range-checked inside the (currently unsupported) `between`. Latent until `between` ships.
- **L3** `src/trader.rs:~373,~392`: a broker `Filled{qty < remaining}` books only `qty` but
  marks the order `Filled`, under-booking vs `req.qty`. Benign with the bundled `SimBroker`;
  guard it before adding a real broker adapter.
- **L5** `src/sim.rs:~1533-1540`: runs `git rev-parse` at runtime for the manifest; spawns a
  process per run in non-git/sandboxed deploys. Reproducibility-only.

## C. Real-world validation (needs the human's Dhan token — not code)
- **§12.1** Run `data bulk --dry-run` and compare the planner's estimated request count
  with reality for ~10 symbols; validates the §3 cost model (1d ≈ 1 req/symbol;
  1m 5y ≈ 21 req/symbol via 89-day chunks). **Still outstanding — report estimate vs actual.**
- **§10 open items:** daily-close settle time (16:00 IST is a placeholder; today's session is
  currently excluded), DH-907 on holidays, the 100k/day reset boundary (assumed IST midnight),
  volume on split days, 375-bar vs 360-bar sessions (downloader must not assume 360).
- **§12 after that:** live 10-symbol check, 100-symbol 1m throughput/memory bench on real
  data, market-hours check on a trading day, full-universe run, then the testing week.

## Suggested order
1. **H1** (small, money-correctness).
2. **Concurrency semaphore** (biggest remaining feature; use the design note; do it with a
   compiler available).
3. **M4 / M5** (sim correctness + perf).
4. Low-severity cleanups (M2, L2, L3, L5) as a batch.
5. Drive the §12/§10 live validation (human-led).
