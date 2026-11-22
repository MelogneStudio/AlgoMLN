# Get All Conditional Orders

`GET` https://api.dhan.co/v2/alerts/orders

Retrieve all conditional orders for the authenticated account.

Retrieve a list of all conditional orders for the authenticated account, along with their current status and configuration details.

Returns every conditional order on your account with its status, condition and the orders it will place.

**Before you start with Get All Conditional Orders API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Paste it into the `access-token` field below.

## Sample request for Get All Conditional Orders API

Fetch all your conditional orders. Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/alerts/orders \
  --header 'access-token: <your-access-token>'
```


## Sample response for Get All Conditional Orders API

```json title="200 OK"
[
  {
    "alertId": "1100000123",
    "alertStatus": "ACTIVE",
    "createdTime": "2026-11-13 09:20:00",
    "triggeredTime": "",
    "lastPrice": 12.97,
    "condition": {
      "comparisonType": "PRICE_WITH_VALUE",
      "exchangeSegment": "NSE_EQ",
      "securityId": "14366",
      "indicatorName": "",
      "timeFrame": "DAY",
      "operator": "CROSSING_UP",
      "comparingValue": 14,
      "comparingIndicatorName": "",
      "expDate": "2026-12-31",
      "frequency": "ONCE",
      "userNote": "dhan-docs-test"
    },
    "orders": [
      {
        "transactionType": "BUY",
        "exchangeSegment": "NSE_EQ",
        "productType": "CNC",
        "orderType": "LIMIT",
        "securityId": "14366",
        "quantity": 1,
        "validity": "DAY",
        "price": "14.05",
        "discQuantity": "0",
        "triggerPrice": "0"
      }
    ]
  }
]
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Status Codes

| Code | Description |
|------|-------------|
| 200 | List of conditional triggers |
| 401 | Unauthorized |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/alerts/orders \
  -H "access-token: <your-access-token>"
```
