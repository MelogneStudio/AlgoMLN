# Market Analytics

Market Analytics: technical indicators, market movers, news headlines and company fundamentals.

Computed technical metrics for an instrument. Only the indicators you request are computed and returned. All values are point-in-time as of the latest closed candle for the requested timeframe.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/data/technical`](/api/v2/market-analytics/fetch-technical-data) | Technical indicator values for an instrument |
| POST | [`/data/companyinfo`](/api/v2/market-analytics/company-info) | Company info metrics for an equity instrument |
| POST | [`/data/marketmovers`](/api/v2/market-analytics/fetch-market-movers) | Top instruments ranked by open interest, volume or price movement |

All enum values on this page are also listed in the [Annexure](/api/v2/annexure#market-analytics).
