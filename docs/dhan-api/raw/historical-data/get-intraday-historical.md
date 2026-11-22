# Get Intraday Historical Data

`POST` https://api.dhan.co/v2/charts/intraday

Get intraday OHLC candle data (1, 5, 15, 25, 60 minute intervals).

Retrieve Open, High, Low, Close, OI & Volume of 1, 5, 15, 25 and 60 min candle for desired instrument for last 5 years. This data available for all exchanges and segments for all active instruments.

> **Note:** The data size is very large in this scenario and only 90 days of data can be polled at once for any of the above time intervals. It is recommended that you store this data at your end for day-to-day analysis.

**Before you start with Get Intraday Historical Data API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Pick the instrument from the [instrument list](/api/v2/instruments), the candle interval and a date range.

## Sample request for Get Intraday Historical Data API

5-minute candles for Vodafone Idea (`14366`) on 13 November 2026. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/charts/intraday \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "14366",
    "exchangeSegment": "NSE_EQ",
    "instrument": "EQUITY",
    "interval": "5",
    "oi": "false",
    "fromDate": "2026-11-13",
    "toDate": "2026-11-14"
  }'
```


## Sample response for Get Intraday Historical Data API

Values are illustrative. Each field is a list with one entry per candle, in the same order. `timestamp` is epoch seconds.

```json title="200 OK"
{
  "open": [
    12.88,
    12.93,
    12.95
  ],
  "high": [
    12.95,
    12.97,
    12.99
  ],
  "low": [
    12.86,
    12.91,
    12.93
  ],
  "close": [
    12.93,
    12.95,
    12.96
  ],
  "volume": [
    18450000,
    9120000,
    7380000
  ],
  "timestamp": [
    1794541500,
    1794541800,
    1794542100
  ],
  "open_interest": [
    0,
    0,
    0
  ]
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| securityId | string | No | Exchange standard ID for each scrip. refer here |
| exchangeSegment | enum | No | Exchange & segment (see Annexure) Values: `NSE_EQ`, `NSE_FNO`, `BSE_EQ`, `BSE_FNO`, `MCX_COMM`, `IDX_I`. |
| instrument | string | No | Instrument type of the scrip (see Annexure) |
| interval | enum | No | Minute intervals Values: `1`, `5`, `15`, `25`, `60`. |
| oi | string | No | Open Interest data for F&O: true or false |
| fromDate | string | No | Start date (YYYY-MM-DD) |
| toDate | string | No | End date (YYYY-MM-DD) |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| open | float | No | Open price of the timeframe |
| high | float | No | High price in the timeframe |
| low | float | No | Low price in the timeframe |
| close | float | No | Close price of the timeframe |
| volume | int | No | Volume traded in the timeframe |
| open_interest | int | No | Open interest (for F&O instruments) |
| timestamp | int | No | Epoch timestamp |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Intraday OHLC data |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/charts/intraday \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
