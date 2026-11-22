# Market Movers

`POST` https://api.dhan.co/v2/data/marketmovers

Fetch top options, futures, and stocks ranked by open interest, traded volume, or price movement. Select the instrument type(s) and ranking category to retrieve the highest-ranked instruments.

Returns the top gainers and losers, the most traded instruments, and the instruments with the highest open interest or the biggest open interest gains and losses, across stocks, futures and options.

**Before you start with Market Movers API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Pick the segment, instrument type and ranking category. Futures and options also need an expiry.

## Sample request for Market Movers API

Top 5 price gainers in the NIFTY 50. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/data/marketmovers \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "exchangeSegment": "NSE_EQ",
    "instrument": [
      "EQUITY"
    ],
    "category": "PRICE_GAINERS",
    "universe": "NIFTY_50",
    "limit": 5
  }'
```


## Sample response for Market Movers API

Values are illustrative. `data` has one entry per instrument, ranked; one is shown here.

```json title="200 OK"
{
  "exchangeSegment": "NSE_EQ",
  "category": "PRICE_GAINERS",
  "data": [
    {
      "securityId": "2885",
      "exchangeSegment": "NSE_EQ",
      "tradingSymbol": "RELIANCE",
      "displayName": "Reliance Industries",
      "instrument": "EQUITY",
      "expiry": null,
      "strikePrice": 0.0,
      "tickSize": 0.1,
      "lotSize": 1,
      "ltp": 1452.6,
      "change": 38.4,
      "changePercent": 2.72,
      "volume": 12456780,
      "tradedValue": 18094580000.0,
      "underlyingSecurityId": "",
      "underlyingLtp": 0.0,
      "premiumDiscount": 0.0,
      "premiumDiscountPercent": 0.0,
      "openInterest": 0,
      "openInterestChange": 0,
      "openInterestChangePercent": 0.0,
      "putCallRatio": 0.0
    }
  ]
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| exchangeSegment | enum | Yes | Exchange segment of the instruments to rank (see Annexure) Values: `NSE_FNO`, `BSE_FNO`, `NSE_COMM`, `MCX_COMM`, `NSE_EQ`, `BSE_EQ`. |
| instrument | array | Yes | Instrument types for which data is to be fetched (see Annexure) Values: `OPTIDX`, `OPTSTK`, `OPTFUT`, `FUTIDX`, `FUTSTK`, `FUTCOM`, `EQUITY`. |
| category | enum | Yes | Ranking category to apply. Supported categories depend on the selected instrument (see Annexure) Values: `HIGHEST_OI`, `OI_GAINERS`, `OI_LOSERS`, `TOP_VOLUME`, `PRICE_GAINERS`, `PRICE_LOSERS`. |
| expiry | string | No | Expiry date of the derivative contracts in YYYY-MM-DD format. Required when instrument is an option or future contract. Optional and ignored when instrument is EQUITY. For eg. "2026-07-28" |
| universe | enum | No | Stock universe within which instruments are ranked, Applicable only when instrument is EQUITY Values: `ALL`, `FNO_STOCKS`, `NIFTY_50`, `NIFTY_BANK`, `FINNIFTY`, `INDIA_VIX`, `NIFTY_MIDCAP`, `NIFTY_NEXT_50`, `NIFTY_SMALLCAP_50`, `NIFTY_MIDCAP_50`, `NIFTY_100`, `NIFTY_200`, `NIFTY_500`, `NIFTY_MIDCAP_100`, `NIFTY_MIDCAP_150`, `NIFTY_SMALLCAP_100`, `NIFTY_SMALLCAP_250`, `NIFTY_MICROCAP_250`, `NIFTY_AUTO`, `NIFTY_PRIVATE_BANK`, `NIFTY_FMCG`, `NIFTY_ENERGY`, `NIFTY_INFRA`, `NIFTY_IT`, `NIFTY_MEDIA`, `NIFTY_METAL`, `NIFTY_MNC`, `NIFTY_PHARMA`, `NIFTY_PSU_BANK`, `NIFTY_REALTY`, `NIFTY_SERVICES_SECTOR`, `NIFTY_CONSUMPTION`, `GIFT_NIFTY`. |
| limit | int | No | Maximum number of instruments to return. Maximum allowed value is 100. For eg. "20" Constraints: 1-100. |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Successful operation |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/data/marketmovers \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
