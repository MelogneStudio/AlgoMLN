# Order Estimator

`POST` https://api.dhan.co/v2/globalstocks/transEstimate

Estimate the applicable charges for a US stock order before submitting it.

Estimate the charges on a US stock order (brokerage, exchange and turnover fees, GST) before you place it.

**Before you start with Order Estimator API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Find the stock's `securityId` in the [US instrument list](/api/v2/instruments#global-stocks). Prices are in US dollars.

## Sample request for Order Estimator API

Charges for buying 1 share of Ford Motor Co (`securityId` 10003455) at $12.30. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/globalstocks/transEstimate \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "10003455",
    "price": "12.30",
    "quantity": "1",
    "transactionType": "BUY"
  }'
```


## Sample response for Order Estimator API

Values are illustrative.

```json title="200 OK"
{
  "brokerage": 0.25,
  "orderCharges": 0.0,
  "exchangeCharges": 0.01,
  "turnOverFee": 0.0,
  "gstCharges": 0.05,
  "otherCharges": 0.0
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| securityId | string | Yes | Exchange standard ID for the scrip |
| price | string | Yes | Order price in USD. Use 0 for market orders. |
| quantity | string | Yes | Order quantity |
| transactionType | enum | Yes | The trading side Values: `BUY`, `SELL`. |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| brokerage | number<float> | No | Brokerage charges |
| orderCharges | number<float> | No | Total order charges |
| exchangeCharges | number<float> | No | Exchange charges |
| turnOverFee | number<float> | No | Turnover fee |
| gstCharges | number<float> | No | GST charges |
| otherCharges | number<float> | No | Other charges |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Transaction estimate |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/globalstocks/transEstimate \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "securityId": "AAPL",
    "price": "180",
    "quantity": "1",
    "transactionType": "BUY"
  }'
```
