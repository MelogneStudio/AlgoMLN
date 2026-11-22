# Margin Calculator

`POST` https://api.dhan.co/v2/globalstocks/margincalculator

Calculate the margin required for a US stock order before submitting it.

Check the margin a US stock order needs and your available balance, before you place it.

**Before you start with Margin Calculator API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Find the stock's `securityId` in the [US instrument list](/api/v2/instruments#global-stocks). Prices are in US dollars.

## Sample request for Margin Calculator API

Margin for buying 1 share of Ford Motor Co (`securityId` 10003455) at $12.30. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/globalstocks/margincalculator \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "securityId": "10003455",
    "price": "12.30",
    "quantity": "1",
    "transactionType": "BUY"
  }'
```


## Sample response for Margin Calculator API

Values are illustrative.

```json title="200 OK"
{
  "brokerage": 0.25,
  "leverage": "1",
  "insufficientBal": 0.0,
  "totalMargin": 12.3,
  "availableBal": 487.7
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
| brokerage | number<float> | No | Brokerage |
| leverage | string | No | Leverage |
| insufficientBal | number<float> | No | Insufficient balance |
| totalMargin | number<float> | No | Total margin required |
| availableBal | number<float> | No | Available balance |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Margin calculation result |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/globalstocks/margincalculator \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "securityId": "AAPL",
    "price": "180",
    "quantity": "1",
    "transactionType": "BUY"
  }'
```
