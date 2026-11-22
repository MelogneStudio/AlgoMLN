# Data APIs

All endpoints for market data, historical charts, option chains, and account statements.

---

## Market Quote

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Get LTP](/api/v2/market-quote/get-ltp) | Get last traded price for instruments |
| POST | [Get OHLC](/api/v2/market-quote/get-ohlc) | Get open, high, low, close data |
| POST | [Get Full Quote](/api/v2/market-quote/get-quote) | Get complete market quote with depth |

## Historical Data

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Daily Historical Data](/api/v2/historical-data/get-daily-historical) | Get daily OHLC candles |
| POST | [Intraday Historical Data](/api/v2/historical-data/get-intraday-historical) | Get intraday candles (1min, 5min, etc.) |

## Option Chain

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Get Expiry List](/api/v2/option-chain/get-expiry-list) | Get available expiry dates |
| POST | [Get Option Chain](/api/v2/option-chain/get-option-chain) | Get full option chain data |

## Market Analytics

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Company Info](/api/v2/market-analytics/company-info) | Fetch selected fundamental metrics for a single equity instrument. |
| POST | [Technical Data](/api/v2/market-analytics/fetch-technical-data) (Pro Traders Plan) | Fetch technical indicator values for an instrument. |
| POST | [Market Movers](/api/v2/market-analytics/fetch-market-movers) (Pro Traders Plan) | Fetch top options, futures, and stocks ranked by open interest, traded volume, or price movement. |
| POST | [Live News Headline](/api/v2/market-analytics/fetch-news-headline) (Pro Traders Plan) | Fetch the latest news headlines as of now, filterable by category and stock, capped at limit (max 50) items. |

## Live Market Data (WebSocket)

| Type | Feed | Description |
|------|------|-------------|
| WebSocket | [Live Market Feed](/api/v2/live-market-feed) | Real-time market data streaming via WebSocket — ticker, quote, and full packets |
| WebSocket | [20 Level Market Depth](/api/v2/20-level-market-depth) | 20 level market depth via WebSocket for NSE Equity and Derivatives |
| WebSocket | [200 Level Market Depth](/api/v2/full-market-depth) (Pro Traders Plan) | 200 level market depth via WebSocket for NSE Equity and Derivatives |
| WebSocket | [CAS Live Feed](/api/v2/cas-live-feed) (Pro Traders Plan) | Closing Auction Session data via WebSocket — indicative equilibrium price, depth and indicative index values |

## Expired Options Data

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Historical Rolling Options Data](/api/v2/expired-options-data/get-expired-options-data) | Fetch expired options data with OHLC, IV, OI |

## Statements

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [Trade History](/api/v2/statements/get-trade-history) | Get detailed trade history |
| GET | [Ledger Report](/api/v2/statements/get-ledger-report) | Get ledger/fund statement |
