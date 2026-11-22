# Get Holdings

`GET` https://api.dhan.co/v2/holdings

Retrieve all holdings in demat account.

Users can retrieve all holdings bought/sold in previous trading sessions. All T1 and delivered quantities can be fetched.

**Before you start with Get Holdings API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Paste it into the `access-token` field below.

## Sample request for Get Holdings API

Fetch your holdings. Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/holdings \
  --header 'access-token: <your-access-token>'
```


## Sample response for Get Holdings API

```json title="200 OK"
[
  {
    "exchange": "ALL",
    "tradingSymbol": "IDEA",
    "securityId": "14366",
    "isin": "INE669E01016",
    "totalQty": 1,
    "dpQty": 1,
    "t1Qty": 0,
    "mtf_t1_qty": 0,
    "mtf_qty": 0,
    "availableQty": 1,
    "collateralQty": 0,
    "avgCostPrice": 12.97,
    "lastTradedPrice": 12.97
  }
]
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| exchange | enum string | No | Exchange Values: `NSE`, `BSE`, `MCX`, `ALL`. |
| tradingSymbol | string | No | Refer Trading Symbol in Tables |
| securityId | string | No | Exchange standard ID for each scrip. refer here |
| isin | string | No | ISIN of the stock |
| totalQty | int | No | T1 qty + Delivered qty |
| dpQty | int | No | Delivered Quantity |
| t1Qty | int | No | T1 Qty |
| mtf_t1_qty | int | No | MTF T1 quantities |
| mtf_qty | int | No | MTF delivered quantities |
| availableQty | int | No | Total quantity - Pledged quantity |
| collateralQty | int | No | Pledged quantity |
| avgCostPrice | float | No | Average buying price of the stock |
| lastTradedPrice | float | No | Last traded price (LTP) for the scrip |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | List of holdings |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/holdings \
  -H "access-token: <your-access-token>"
```
