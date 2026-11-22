# Company Info

`POST` https://api.dhan.co/v2/data/companyinfo

Fetch selected fundamental metrics for a single equity instrument. Pass the required metric groups and receive the latest available snapshot across company classification, industry benchmarks, shareholding, valuation, profitability, price performance, and book value.

Company information for a stock is available in three sections: the company overview, which includes market cap, sector, EPS, EBITDA, book value and the 52-week range; the return ratios, such as P/E, P/B, ROE and debt to equity; and the shareholding pattern.

**Before you start with Company Info API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Pick the stock and the metric sections you need. Only those are returned.

## Sample request for Company Info API

Company overview (`CO`) for Vodafone Idea (`14366`). Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/data/companyinfo \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "14366",
    "exchangeSegment": "NSE_EQ",
    "instrument": "EQUITY",
    "metrics": [
      "CO"
    ]
  }'
```


## Sample response for Company Info API

Values are illustrative.

```json title="200 OK"
{
  "securityId": "14366",
  "data": {
    "CO": {
      "MARKET_CAP": "140520.6",
      "INDUSTRY": "Telecom - Cellular & Fixed line services",
      "SECTOR": "Telecommunication",
      "DIVIDEND_YIELD": "0",
      "EPS": "-2.61",
      "EBITDA": "18240",
      "BOOK_VALUE": "-6.48",
      "FIFTY_TWO_WEEK_HIGH": "15.79",
      "FIFTY_TWO_WEEK_LOW": "8.13"
    }
  }
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| securityId | string | Yes | Exchange standard ID for each security. For eg. "1333" |
| exchangeSegment | enum | Yes | Exchange segment of the instrument NSE_EQ and BSE_EQ (see Annexure) Values: `NSE_EQ`, `BSE_EQ`. |
| instrument | enum | Yes | Instrument Type EQUITY (see Annexure) Values: `EQUITY`. |
| metrics | array | Yes | Metric sections to fetch. Only those passed here are returned. For eg. [ "CO" ] (see Annexure) Values: `CO`, `RATIOS`, `SHP`. |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Successful operation |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/data/companyinfo \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
