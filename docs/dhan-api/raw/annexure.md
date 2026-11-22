# Annexure

Complete reference for all enum values, exchange segments, product types, error codes and more

Reference tables for all enum values used across DhanHQ APIs.

---

## Exchange Segment

| Attribute | Exchange | Segment | Enum |
|-----------|----------|---------|------|
| `IDX_I` | Index | Index Value | `0` |
| `NSE_EQ` | NSE | Equity Cash | `1` |
| `NSE_FNO` | NSE | Futures & Options | `2` |
| `BSE_EQ` | BSE | Equity Cash | `4` |
| `MCX_COMM` | MCX | Commodity | `5` |
| `BSE_FNO` | BSE | Futures & Options | `8` |

---

## Product Type

| Attribute | Detail |
|-----------|--------|
| `CNC` | Cash & Carry for equity deliveries |
| `INTRADAY` | Intraday for Equity, Futures & Options |
| `MARGIN` | Carry Forward in Futures & Options |
| `MTF` | Margin Trading Facility |

---

## Order Status

| Attribute | Detail |
|-----------|--------|
| `TRANSIT` | Did not reach the exchange server |
| `PENDING` | Awaiting execution |
| `CLOSED` | Used for Super Order, once both entry and exit orders are placed |
| `TRIGGERED` | Used for Super Order, if Target or Stop Loss leg is triggered |
| `REJECTED` | Rejected by broker/exchange |
| `CANCELLED` | Cancelled by user |
| `PART_TRADED` | Partial Quantity traded successfully |
| `TRADED` | Executed successfully |

---

## After Market Order Time

| Attribute | Detail |
|-----------|--------|
| `PRE_OPEN` | AMO pumped at pre-market session |
| `OPEN` | AMO pumped at market open |
| `OPEN_30` | AMO pumped 30 minutes after market open |
| `OPEN_60` | AMO pumped 60 minutes after market open |

---

## Expiry Code

| Attribute | Detail |
|-----------|--------|
| `1` | Near Expiry |
| `2` | Next Expiry |
| `3` | Far Expiry |

---

## Instrument

| Attribute | Detail |
|-----------|--------|
| `INDEX` | Index |
| `FUTIDX` | Futures of Index |
| `OPTIDX` | Options of Index |
| `EQUITY` | Equity |
| `FUTSTK` | Futures of Stock |
| `OPTSTK` | Options of Stock |
| `FUTCOM` | Futures of Commodity |
| `OPTFUT` | Options of Commodity Futures |

---

## Feed Request Code

| Code | Action |
|------|--------|
| `11` | Connect Feed |
| `12` | Disconnect Feed |
| `15` | Subscribe — Ticker Packet |
| `16` | Unsubscribe — Ticker Packet |
| `17` | Subscribe — Quote Packet |
| `18` | Unsubscribe — Quote Packet |
| `21` | Subscribe — Full Packet |
| `22` | Unsubscribe — Full Packet |
| `23` | Subscribe — Full Market Depth |
| `25` | Unsubscribe — Full Market Depth |

---

## Feed Response Code

| Code | Packet |
|------|--------|
| `1` | Index Packet |
| `2` | Ticker Packet |
| `4` | Quote Packet |
| `5` | OI Packet |
| `6` | Prev Close Packet |
| `7` | Market Status Packet |
| `8` | Full Packet |
| `50` | Feed Disconnect |

---

## Trading API Error

| Type | Code | Message |
|------|------|---------|
| Invalid Authentication | `DH-901` | Client ID or user generated access token is invalid or expired |
| Invalid Access | `DH-902` | User has not subscribed to Data APIs or does not have access to Trading APIs |
| User Account | `DH-903` | Errors related to User's Account — check if required segments are activated |
| Rate Limit | `DH-904` | Too many requests — try throttling API calls |
| Input Exception | `DH-905` | Missing required fields, bad values for parameters etc. |
| Order Error | `DH-906` | Incorrect request for order — cannot be processed |
| Data Error | `DH-907` | Unable to fetch data due to incorrect parameters or no data present |
| Internal Server Error | `DH-908` | Server was not able to process request — occurs rarely |
| Network Error | `DH-909` | API was unable to communicate with backend system |
| Others | `DH-910` | Error originating from other reasons |

---

## Data API Error

| Code | Description |
|------|-------------|
| `800` | Internal Server Error |
| `804` | Requested number of instruments exceeds limit |
| `805` | Too many requests or connections — further requests may result in blocking |
| `806` | Data APIs not subscribed |
| `807` | Access token is expired |
| `808` | Authentication Failed — Client ID or Access Token invalid |
| `809` | Access token is invalid |
| `810` | Client ID is invalid |
| `811` | Invalid Expiry Date |
| `812` | Invalid Date Format |
| `813` | Invalid SecurityId |
| `814` | Invalid Request |

---

## Conditional and Multi Order

### Comparison Type

| Type | Description | Mandatory Fields |
|------|-------------|------------------|
| `TECHNICAL_WITH_VALUE` | Compare technical indicator against a fixed numeric value | `indicatorName` `operator` `timeFrame` `comparingValue` |
| `TECHNICAL_WITH_INDICATOR` | Compare technical indicator against another indicator | `indicatorName` `operator` `timeFrame` `comparingIndicatorName` |
| `TECHNICAL_WITH_CLOSE` | Compare a technical indicator with closing price | `indicatorName` `operator` `timeFrame` |
| `PRICE_WITH_VALUE` | Compare market price against fixed value | `operator` `comparingValue` |

### Indicator Name

| Indicator | Description |
|-----------|-------------|
| `SMA_5` | Simple Moving Average (5 periods) |
| `SMA_10` | Simple Moving Average (10 periods) |
| `SMA_20` | Simple Moving Average (20 periods) |
| `SMA_50` | Simple Moving Average (50 periods) |
| `SMA_100` | Simple Moving Average (100 periods) |
| `SMA_200` | Simple Moving Average (200 periods) |
| `EMA_5` | Exponential Moving Average (5 periods) |
| `EMA_10` | Exponential Moving Average (10 periods) |
| `EMA_20` | Exponential Moving Average (20 periods) |
| `EMA_50` | Exponential Moving Average (50 periods) |
| `EMA_100` | Exponential Moving Average (100 periods) |
| `EMA_200` | Exponential Moving Average (200 periods) |
| `BB_UPPER` | Upper Bollinger Band |
| `BB_LOWER` | Lower Bollinger Band |
| `RSI_14` | Relative Strength Index |
| `ATR_14` | Average True Range |
| `STOCHASTIC` | Stochastic Oscillator |
| `STOCHRSI_14` | Stochastic RSI |
| `MACD_26` | MACD long-term component |
| `MACD_12` | MACD short-term component |
| `MACD_HIST` | MACD histogram |

### Operator

| Operator | Description |
|----------|-------------|
| `CROSSING_UP` | Crosses above |
| `CROSSING_DOWN` | Crosses below |
| `CROSSING_ANY_SIDE` | Crosses either side |
| `GREATER_THAN` | Greater than |
| `LESS_THAN` | Less than |
| `GREATER_THAN_EQUAL` | Greater than or equal |
| `LESS_THAN_EQUAL` | Less than or equal |
| `EQUAL` | Equal |
| `NOT_EQUAL` | Not equal |

### Status

| Status | Description |
|--------|-------------|
| `ACTIVE` | Alert is currently active |
| `TRIGGERED` | Alert condition met |
| `EXPIRED` | Alert expired |
| `CANCELLED` | Alert cancelled |

---

## Market Analytics

### Timeframe

Candle size used to compute all indicators in a request.

| Value | Type | Description |
|-------|------|-------------|
| `1` | Intraday | 1-minute candles |
| `5` | Intraday | 5-minute candles |
| `15` | Intraday | 15-minute candles |
| `D` | Daily | End-of-day daily candles. Default for positional analysis. |

### Technical Indicators — Moving Averages

| Identifier | Description |
|------------|-------------|
| `SMA_5` | Simple Moving Average — 5-period |
| `SMA_10` | Simple Moving Average — 10-period |
| `SMA_20` | Simple Moving Average — 20-period |
| `SMA_50` | Simple Moving Average — 50-period |
| `SMA_100` | Simple Moving Average — 100-period |
| `SMA_200` | Simple Moving Average — 200-period |
| `EMA_5` | Exponential Moving Average — 5-period |
| `EMA_10` | Exponential Moving Average — 10-period |
| `EMA_20` | Exponential Moving Average — 20-period |
| `EMA_50` | Exponential Moving Average — 50-period |
| `EMA_100` | Exponential Moving Average — 100-period |
| `EMA_200` | Exponential Moving Average — 200-period |

### Technical Indicators — Momentum / Oscillators

| Identifier | Description | Fixed Parameters |
|------------|-------------|------------------|
| `RSI_14` | Relative Strength Index | Period: 14 |
| `MACD_HIST` | MACD histogram (MACD line minus Signal line) | Fast: 12 / Slow: 26 / Signal: 9 |
| `STOCH` | Stochastic Oscillator | K: 9 / D: 6 |
| `STOCHRSI_14` | Stochastic RSI | Period: 14 |
| `ATR_14` | Average True Range | Period: 14 |
| `ADX_14` | Average Directional Index — trend strength, not direction | Period: 14 |
| `UO` | Ultimate Oscillator | Periods: 7 / 14 / 28 |
| `ROC` | Rate of Change | Period: 12 |
| `WILLR` | Williams %R | Period: 14 |

### Technical Indicators — Pivot Points

| Identifier | Description |
|------------|-------------|
| `PIVOT_CLASSIC` | Standard floor pivot. Levels computed from previous session OHLC. |
| `PIVOT_FIBONACCI` | Fibonacci ratios applied to the pivot range. |
| `PIVOT_CAMARILLA` | Tighter intraday levels derived from close and previous range. |

### Company Info Metrics

Metric sections available to the Company Info API. Only the sections passed in the request are returned.

| Value | Description |
|-------|-------------|
| `CO` | Company Overview |
| `RATIOS` | Return Ratios |
| `SHP` | Shareholding Pattern |

#### Response Parameters — CO (Company Overview)

| Parameters | Data Type | Description | Sample value |
|------------|-----------|-------------|--------------|
| `MARKET_CAP` | String | Market capitalisation in Rs Cr. | 36318.4 |
| `INDUSTRY` | String | Specific industry in which the company operates. | Cement |
| `SECTOR` | String | Broad economic sector to which the company belongs. | Construction |
| `DIVIDEND_YIELD` | String | Annual dividend per share as a percentage of the current market price. | 0.21 |
| `EPS` | String | Earnings attributable to each outstanding equity share, based on the trailing twelve months. | 60.7 |
| `EBITDA` | String | Earnings before interest, tax, depreciation, and amortisation for the trailing twelve months, in ₹ crore. | 3278 |
| `BOOK_VALUE` | String | Latest book value per equity share, in ₹. | 958.52 |
| `FIFTY_TWO_WEEK_HIGH` | String | Highest price at which the security traded during the previous 52 weeks. | 1245 |
| `FIFTY_TWO_WEEK_LOW` | String | Lowest price at which the security traded during the previous 52 weeks. | 865.5 |

#### Response Parameters — RATIOS (Return Ratios)

| Parameters | Data Type | Description | Sample value |
|------------|-----------|-------------|--------------|
| `PE_RATIO` | String | Price-to-Earnings ratio of the company based on trailing twelve-month earnings. | 31.2 |
| `PB_RATIO` | String | Price-to-Book ratio of the company. | 2.01 |
| `DEBT_TO_EQUITY` | String | Ratio of the company's total debt to shareholders' equity. | 0.65 |
| `PE_TO_SECTOR_PE` | String | Ratio of the company's PE ratio to the average PE ratio of its sector. | 0.82 |
| `ROE` | String | Return on Equity, indicating how efficiently the company generates profit using shareholders' equity, expressed as a percentage. | 3.99 |
| `ROCE` | String | Return on Capital Employed, indicating how efficiently the company generates operating profit using the capital employed, expressed as a percentage. | 4.86 |
| `INDUSTRY_EPS` | String | Average earnings per share of companies operating in the same industry. | 42.6 |
| `INDUSTRY_DEBT_TO_EQUITY` | String | Average debt-to-equity ratio of companies operating in the same industry. | 0.85 |
| `INDUSTRY_PB_RATIO` | String | Average price-to-book ratio of companies operating in the same industry. | 4.2 |
| `INDUSTRY_CURRENT_RATIO` | String | Average current ratio of companies operating in the same industry. | 1.75 |
| `INDUSTRY_DIVIDEND_YIELD` | String | Average dividend yield of companies operating in the same industry, expressed as a percentage. | 1.4 |
| `INDUSTRY_OPERATING_MARGIN` | String | Average operating profit margin of companies operating in the same industry, expressed as a percentage. | 18.6 |

#### Response Parameters — SHP (Shareholding Pattern)

| Parameters | Data Type | Description | Sample value |
|------------|-----------|-------------|--------------|
| `REPORTING_PERIOD` | String | Latest quarter for which shareholding data is reported, in YYYYMM format. | 202603 |
| `PROMOTER_HOLDING` | String | Percentage of the company's total shares held by promoters and the promoter group. | 55.84 |
| `DII_HOLDING` | String | Percentage of the company's total shares held by domestic institutional investors. | 20.25 |
| `CHANGE_IN_DII_HOLDING` | String | Change in DII holding compared with the previous reported quarter, in percentage points. | 1.26 |
| `FII_HOLDING` | String | Percentage of the company's total shares held by foreign institutional investors. | 7.15 |
| `CHANGE_IN_FII_HOLDING` | String | Change in FII holding compared with the previous reported quarter, in percentage points. | -0.77 |
| `PUBLIC_HOLDING` | String | Percentage of the company's total shares held by public or retail shareholders. | 8.06 |

### News Category

News categories accepted by the Live News Headline API. `ALL` returns all categories, unfiltered.

| Value | Description |
|-------|-------------|
| `ALL` | All categories, unfiltered |
| `COMPANIES` | Companies |
| `EQUITY_MARKETS` | Equity markets |
| `DEBT_MARKETS` | Debt markets |
| `IPO` | IPO |
| `GLOBAL` | Global |
| `INDIAN_ECONOMY` | Indian economy |
| `GLOBAL_ECONOMY` | Global economy |
| `CURRENCY` | Currency |
| `COMMODITIES` | Commodities |
| `CRYPTOCURRENCIES` | Cryptocurrencies |
| `GOVERNMENT_POLICY_AND_REGULATION` | Government policy and regulation |
| `INFRASTRUCTURE` | Infrastructure |
| `INTERNATIONAL_TRADE` | International trade |
| `INVESTMENT_IDEAS` | Investment ideas |
| `REAL_ESTATE` | Real estate |
| `STARTUPS` | Startups |
| `TECHNOLOGY` | Technology |
| `MUTUAL_FUNDS` | Mutual funds |

### Market Movers Instrument

Instrument types accepted by the Market Movers API. All values in a single request must belong to the same group (options, futures, or equity) — they cannot be mixed.

| Value | Group | Description |
|-------|-------|-------------|
| `OPTIDX` | Options | Options of Index |
| `OPTSTK` | Options | Options of Stock |
| `OPTFUT` | Options | Options of Commodity Futures |
| `FUTIDX` | Futures | Futures of Index |
| `FUTSTK` | Futures | Futures of Stock |
| `FUTCOM` | Futures | Futures of Commodity |
| `EQUITY` | Stocks | Equity |

### Ranking Category

Supported categories depend on the selected instrument.

| Value | Applicable To | Description |
|-------|---------------|-------------|
| `HIGHEST_OI` | Options | Highest open interest |
| `OI_GAINERS` | Options, Futures | Largest increase in open interest |
| `OI_LOSERS` | Options, Futures | Largest decrease in open interest |
| `TOP_VOLUME` | Options, Futures | Highest traded volume |
| `PRICE_GAINERS` | Stocks | Largest price increase |
| `PRICE_LOSERS` | Stocks | Largest price decrease |
