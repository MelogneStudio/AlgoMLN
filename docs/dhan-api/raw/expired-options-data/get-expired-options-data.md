# Historical Rolling Options Data

`POST` https://api.dhan.co/v2/charts/rollingoption

Fetch expired options contract data on a rolling basis with OHLC, IV, OI and spot information. Up to one day per call. Last 5 years available.

Fetch expired options data on a rolling basis, along with the Open Interest, Implied Volatility, OHLC, Volume as well as information about the spot. You can fetch up to one day of data in a single API call. Expired options data is stored on a minute level, based on strike price relative to spot (example ATM, ATM+10, ATM-10, etc.).

You can fetch data upto last 5 years. We have added both Index Options and Stock Options data on this.

**Before you start with Historical Rolling Options Data API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Choose the underlying, expiry (week or month), strike (for example `ATM` or `ATM+1`) and option type.

## Sample request for Historical Rolling Options Data API

1-minute data for the near-week NIFTY (`13`) ATM call on 27 October 2026. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/charts/rollingoption \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "exchangeSegment": "NSE_FNO",
    "interval": "1",
    "securityId": "13",
    "instrument": "OPTIDX",
    "expiryFlag": "WEEK",
    "expiryCode": "1",
    "strike": "ATM",
    "drvOptionType": "CALL",
    "requiredData": [
      "open",
      "high",
      "low",
      "close",
      "iv",
      "volume",
      "oi",
      "spot",
      "strike"
    ],
    "fromDate": "2026-10-27",
    "toDate": "2026-10-28"
  }'
```


## Sample response for Historical Rolling Options Data API

Values are illustrative. `pe` is `null` because a call was requested. Each field is a list with one entry per candle.

```json title="200 OK"
{
  "data": {
    "ce": {
      "open": [
        142.5,
        140.1
      ],
      "high": [
        145.0,
        141.8
      ],
      "low": [
        139.6,
        138.9
      ],
      "close": [
        140.2,
        139.4
      ],
      "volume": [
        1854000,
        1126500
      ],
      "iv": [
        13.8,
        13.7
      ],
      "oi": [
        6234500,
        6301250
      ],
      "spot": [
        25812.4,
        25806.9
      ],
      "strike": [
        25800.0,
        25800.0
      ],
      "timestamp": [
        1793072700,
        1793072760
      ]
    },
    "pe": null
  }
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| exchangeSegment | enum | No | Exchange & Segment (see Annexure) Values: `NSE_FNO`, `BSE_FNO`. |
| interval | enum | No | Minute intervals Values: `1`, `5`, `15`, `25`, `60`. |
| securityId | string | No | Underlying security ID |
| instrument | enum | No | Instrument type (see Annexure) Values: `OPTIDX`, `OPTSTK`. |
| expiryFlag | enum | No | Expiry Flag Values: `WEEK`, `MONTH`. |
| expiryCode | enum | No | 1=Near, 2=Next, 3=Far Values: `1`, `2`, `3`. |
| strike | string | No | ATM for At the Money<br/>Upto ATM+10 / ATM-10 for Index Options near expiry<br/>Upto ATM+3 / ATM-3 for all other contracts |
| drvOptionType | enum | No | Option Type Values: `CALL`, `PUT`. |
| requiredData | array | No | Array of data fields: open, high, low, close, iv, volume, strike, oi, spot Values: `open`, `high`, `low`, `close`, `iv`, `volume`, `strike`, `oi`, `spot`. |
| fromDate | string | No | Start Date (YYYY-MM-DD) |
| toDate | string | No | End Date (YYYY-MM-DD) |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| data.ce.open | array | No | Array of open prices for CE |
| data.ce.high | array | No | Array of high prices for CE |
| data.ce.low | array | No | Array of low prices for CE |
| data.ce.close | array | No | Array of close prices for CE |
| data.ce.volume | array | No | Array of volumes for CE |
| data.ce.iv | array | No | Array of implied volatilities for CE |
| data.ce.oi | array | No | Array of open interest for CE |
| data.ce.spot | array | No | Array of underlying spot prices for CE |
| data.ce.strike | array | No | Array of strike prices for CE |
| data.ce.timestamp | array | No | Array of epoch timestamps for CE |
| data.pe | object | No | Object containing PE data points analogous to CE |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Expired options data |
| 401 | Unauthorized |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/charts/rollingoption \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
