# Indicator golden fixtures

These CSVs contain one output row per source candle, with blank/`None` cells
for warmup positions. Regenerate them from the bundled OHLC input with:

```sh
python scripts/generate_indicator_fixtures.py
```

The source data has no volume column, so relative-volume and VWAP fixtures are
all undefined. Volume behavior is covered with small explicit synthetic cases
in the Rust tests. Five source rows also have a close that differs from the
reported low/high by less than 0.001; the fixture harness adjusts those bounds
to include close before provider validation. The generator uses rolling
population standard deviation computed from each window for Bollinger values.
