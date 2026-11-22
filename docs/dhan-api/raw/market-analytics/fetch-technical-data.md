# Technical Data

`POST` https://api.dhan.co/v2/data/technical

Fetch technical indicator values for an instrument. Pass the indicators you need - only those are computed and returned. All values are point-in-time as of the latest closed candle for the requested timeframe.

Technical indicators such as SMA, EMA, RSI, MACD, ADX, ATR, Stochastic and pivot points can be fetched for a stock or an index. They are computed on 1, 5 or 15 minute candles or on daily candles, depending on the timeframe you choose.

**Before you start with Technical Data API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Pick the instrument, candle size and the indicators you need. Only those are returned.

## Sample request for Technical Data API

20-day SMA and 14-day RSI for Vodafone Idea (`14366`) on daily candles. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/data/technical \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "14366",
    "exchangeSegment": "NSE_EQ",
    "instrument": "EQUITY",
    "timeframe": "D",
    "indicators": [
      "SMA_20",
      "RSI_14"
    ]
  }'
```


## Sample response for Technical Data API

Values are illustrative. `data` has one entry per requested indicator; the fields inside each entry are not documented yet, so `value` here is a placeholder.

```json title="200 OK"
{
  "securityId": "14366",
  "timeframe": "D",
  "data": {
    "SMA_20": {
      "value": 12.84
    },
    "RSI_14": {
      "value": 58.6
    }
  }
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| securityId | string | Yes | Exchange standard ID for each security. For eg. "1333" |
| exchangeSegment | enum | Yes | Exchange segment of the instrument (see Annexure) Values: `NSE_EQ`, `IDX_I`. |
| instrument | enum | Yes | Valid Instrument types INDEX and EQUITY (see Annexure) Values: `INDEX`, `EQUITY`. |
| timeframe | enum | Yes | Candle size used to compute all indicators in this request (see Annexure) Values: `1`, `5`, `15`, `D`. |
| indicators | array | Yes | Indicator identifiers to fetch. Only those passed here are computed and returned. For eg. [ "SMA_20", "RSI_14", "MACD_HIST" ] (see Annexure) Values: `SMA_5`, `SMA_10`, `SMA_20`, `SMA_50`, `SMA_100`, `SMA_200`, `EMA_5`, `EMA_10`, `EMA_20`, `EMA_50`, `EMA_100`, `EMA_200`, `RSI_14`, `MACD_HIST`, `STOCH`, `STOCHRSI_14`, `ATR_14`, `ADX_14`, `UO`, `ROC`, `WILLR`, `PIVOT_CLASSIC`, `PIVOT_FIBONACCI`, `PIVOT_CAMARILLA`. |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Successful operation |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/data/technical \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
