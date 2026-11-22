# Historical Data

Historical Data: daily and intraday (1 to 60-minute) OHLC candles for charts and backtesting.

This API gives you historical candle data for the desired scrip across segments & exchange. This data is presented in the form of a candle and gives you timestamp, open, high, low, close & volume.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/charts/historical`](/api/v2/historical-data/get-daily-historical) | Get OHLC for daily timeframe |
| POST | [`/charts/intraday`](/api/v2/historical-data/get-intraday-historical) | Get OHLC for minute timeframe |
