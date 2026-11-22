# Expired Options Data

Expired Options Data: minute-level OHLC, IV and OI for expired index and stock options.

This API gives you expired options contract data. We have pre processed data for you to get it on rolling basis i.e. you can fetch last 5 years of strike wise data based on ATM and upto 10 strikes above and below. In addition to that, the data values are open, high, low, close, implied volatility, volume, open interest and spot information as well.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/charts/rollingoption`](/api/v2/expired-options-data/get-expired-options-data) | Get Continuous Expired Options Contract data |
