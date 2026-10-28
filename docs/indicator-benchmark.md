# Incremental indicator benchmark

Run with `cargo bench --bench indicators -- --sample-size 10`.

Recorded 2026-10-28 on Windows, Intel Core i5-1145G7 (4 cores / 8 threads),
Rust 1.98.0. Criterion processed the 184,863-row NIFTY OHLC input with the
full registered set (SMA 20, EMA 20, RSI 14, ATR 14, three Bollinger 20
outputs, relative volume 20, and VWAP):

- Estimated throughput: **8.21 million candles/s** (Criterion interval
  6.57–9.24 million candles/s).
- Estimated iteration time: **22.5 ms** (interval 20.0–28.1 ms).

The benchmark includes provider construction and indicator registration in
each iteration; CSV parsing is outside the measured region. The input has no
volume column, so this run exercises zero-volume volume indicators. The result
is a baseline for this machine, not a cross-machine performance guarantee.
