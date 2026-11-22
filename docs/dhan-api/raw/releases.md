# Release Notes

Changelog and release notes for DhanHQ API versions

---

## Version 2.6
*Date: Monday Aug 10, 2026*

Introducing a new Market Analytics section under Data APIs, and CAS Live Feed for the Closing Auction Session.

### New Features

- **Market Analytics** — New Data API section with computed metrics for instruments. [Read more →](/api/v2/market-analytics)
- **Technical Data** (`POST /data/technical`) — Technical indicator values for an instrument, computed as of the latest closed candle. [Read more →](/api/v2/market-analytics/fetch-technical-data)
- **Company Info** (`POST /data/companyinfo`) — Company overview, return ratios and shareholding pattern metrics for an equity instrument. [Read more →](/api/v2/market-analytics/company-info)
- **Market Movers** (`POST /data/marketmovers`) — Top instruments ranked by open interest, volume or price movement. [Read more →](/api/v2/market-analytics/fetch-market-movers)
- **Live News Headline** (`POST /data/newsheadline`) — Latest news headlines, filterable by category and stock. [Read more →](/api/v2/market-analytics/fetch-news-headline)

- **CAS Live Feed** — Stream Closing Auction Session data over WebSocket. [Read more →](/api/v2/cas-live-feed)

---

## Version 2.5
*Date: Monday Feb 09, 2026*

Introducing Conditional and Multi Order, P&L based exit under Trader's Control, and Exit All API.

### New Features

- **Conditional and Multi Order** — Place orders triggered when specific market conditions are met, allowing automation of trading strategies. [Read more →](/api/v2/conditional-triggers)

- **P&L Based Exit** — Set profit or loss thresholds for automatic position closure. [Read more →](/api/v2/traders-control/configure-pnl-exit)

- **Exit All API** — Close all open positions and orders with a single API call.

- **Access Token Generation via API** — Generate access tokens via API with TOTP configured. [Read more →](/api/v2/authentication/generate-token)

### Improvements

- **Option Chain API** — Enhanced rate limits for multiple unique requests. Added `average_price` and `security_id` fields.

---

## Version 2.4
*Date: Monday Sep 22, 2025*

Changes in line with SEBI guidelines on Retail Participation in Algorithmic Trading — reduced access token duration, API key based authentication, and IP setup.

### New Features

- **API Key Based Login** — New authentication module using API key and secret for one-year validity. [Read more →](/api/v2/authentication#api-key--secret)

### Breaking Changes

- **Access Token limited to 24 hours** — In line with exchange and SEBI guidelines.
- **Static IP Requirement** — Required for all Order APIs. [Setup Static IP →](/api/v2/authentication#setup-static-ip)

---

## Version 2.3
*Date: Monday Sep 08, 2025*

Adding 200 Market Depth on WebSockets and historical expired options contract data. Rate limit for Order APIs changed to 10 orders per second.

### New Features

- **Full Market Depth (200 levels)** — Extension of 20-level depth with complete order book data on WebSocket. [Read more →](/api/v2/full-market-depth)

- **Historical Options Data** — Fetch expired contract data on a rolling basis with IV, OI and volumes. [Read more →](/api/v2/expired-options-data)

---

## Version 2.2
*Date: Friday Mar 07, 2025*

New Super Order type and major updates to Historical Data APIs — 5 years of intraday data and OI data.

### New Features

- **Super Orders** — Combine entry and exit orders into a single order with trailing stop loss, available across all exchanges and segments.

- **User Profile** — Status check API for token validity, active segments, Data API subscription and more. [Read more →](/api/v2/authentication#user-profile)

### Improvements

- **Intraday Historical Data** — Now available for last 5 years with OI data. Added `oi` parameter and IST time support in date parameters.

- **Daily Historical Data** — Added OI data with optional `oi` parameter.

- **`CorrelationId` on Live Order Update** — New `CorrelationId` and `Remarks` keys. [Read more →](/api/v2/order-update#order-update)

### Breaking Changes

- **Rate Limit Changes** — Increased for Data APIs. No rate limits on minute/hourly timeframes. 1,00,000 daily requests, seconds limited to 5 per second.

---

## Version 2.1
*Date: Monday Jan 06, 2025*

Adds 20-level market depth (Level 3 data) and Option Chain API.

### New Features

- **20 Market Depth** — Real-time streaming of 20-level depth for all NSE instruments. [Read more →](/api/v2/20-level-market-depth)

- **Option Chain** — Advanced Option Chain with OI, Greeks, volume, top bid/ask and price data for all strikes.

### Improvements

- **`expiryCode` in Daily Historical Data** — Now available as an optional field.

---

## Version 2.0
*Date: Monday Sep 15, 2024*

DhanHQ v2 extends execution capability with live order updates, Market Quote APIs, and stability improvements.

### New Features

- **Market Quote** — Fetch LTP, Quote (with OI) and Market Depth for up to 1000 instruments at once.

- **Live Order Update** — Real-time order status updates via WebSocket. [Read more →](/api/v2/order-update)

- **Margin Calculator** — Details about required margin and available balances before placing orders.

### Improvements

- **Intraday Historical Data** — OHLC with Volume for last 5 trading days across multiple timeframes.

- **GET Order APIs** — Added `filledQty`, `remainingQuantity`, `averageTradedPrice` and `PART_TRADED` status.

- **Live Market Feed** — Authorise via Query Parameters. `FULL` packet now available with combined data.

### Breaking Changes

- **Order Placement** — Deprecated `tradingSymbol`, `drvExpiryDate`, `drvOptionType`, `drvStrikePrice`. Added `PRE_OPEN` AMO tag.

- **Order Modification** — `quantity` field now requires placed quantity instead of pending quantity.

- **Daily Historical Data** — `symbol` replaced with `securityId`.

- **Error Messages** — Categorised with DH-900 series.

- **Security ID List** — New comprehensive mapping with tag changes. [See Instruments →](/api/v2/instruments)

- **Epoch time** — Replaced Julian time in Historical Data APIs.

- **Market Depth deprecated** — Replaced with `FULL` packet in Live Market Feed.

- **Endpoint changes** — Trade History now `/trades`, Kill Switch now `/killswitch`.

### Bug Fixes

- **Positions API** — `realizedProfit` and `unrealizedProfit` now return real-time values.

- **Order Modification** — `TARGET_LEG` modification fixed.
