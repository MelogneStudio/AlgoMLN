# Get Daily Historical Data

`POST` https://api.dhan.co/v2/charts/historical

Get daily OHLC candle data for backtesting and analysis.

Retrieve OHLC & Volume of daily candle for desired instrument. The data for any scrip is available back upto the date of its inception.

You provide the security ID, exchange segment and instrument type of the scrip, along with a start date and an end date. The end date is non-inclusive. For F&O instruments, Open Interest data can also be fetched, and the expiry code can be passed for derivatives.

**Before you start with Get Daily Historical Data API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Pick the instrument from the [instrument list](/api/v2/instruments) and a date range.

## Sample request for Get Daily Historical Data API

Daily candles for Vodafone Idea (`14366`) from 10 to 12 November 2026 (`toDate` is not included). Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/charts/historical \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "14366",
    "exchangeSegment": "NSE_EQ",
    "instrument": "EQUITY",
    "expiryCode": 0,
    "oi": "false",
    "fromDate": "2026-11-10",
    "toDate": "2026-11-13"
  }'
```


## Sample response for Get Daily Historical Data API

Values are illustrative. Each field is a list with one entry per candle, in the same order. `timestamp` is epoch seconds.

```json title="200 OK"
{
  "open": [
    12.7,
    12.81,
    12.88
  ],
  "high": [
    12.92,
    12.95,
    13.05
  ],
  "low": [
    12.62,
    12.74,
    12.84
  ],
  "close": [
    12.81,
    12.88,
    12.97
  ],
  "volume": [
    288140000,
    301220000,
    312450000
  ],
  "timestamp": [
    1794249000,
    1794335400,
    1794421800
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
| expiryCode | int | No | Expiry of the instrument (for derivatives) (see Annexure) |
| oi | string | No | Open Interest data for F&O: true or false |
| fromDate | string | No | Start date (YYYY-MM-DD) |
| toDate | string | No | End date (YYYY-MM-DD, non-inclusive) |


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
| 200 | Historical OHLC data |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/charts/historical \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
