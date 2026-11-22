# Get OHLC

`POST` https://api.dhan.co/v2/marketfeed/ohlc

Get OHLC (Open, High, Low, Close) data for instruments.

Retrieve the Open, High, Low and Close price along with LTP for specified list of instruments.

> **Info:** You can fetch upto 1000 instruments in single API request with rate limit of 1 request per second.

**Before you start with Get OHLC API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Add security IDs under each exchange segment. Find them in the [instrument list](/api/v2/instruments).


## Sample request for Get OHLC API

OHLC of Vodafone Idea on NSE (`14366`), its December futures contract on NSE F&O (`58992`) and its BSE equity (`532822`). Replace the client ID and access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/marketfeed/ohlc \
  --header 'Content-Type: application/json' \
  --header 'client-id: <your-client-id>' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "NSE_EQ": [
      14366
    ],
    "NSE_FNO": [
      58992
    ],
    "BSE_EQ": [
      532822
    ]
  }'
```


## Sample response for Get OHLC API

Values are illustrative.

```json title="200 OK"
{
  "data": {
    "NSE_EQ": {
      "14366": {
        "last_price": 12.97,
        "ohlc": {
          "open": 12.88,
          "high": 13.05,
          "low": 12.84,
          "close": 12.88
        }
      }
    },
    "NSE_FNO": {
      "58992": {
        "last_price": 13.05,
        "ohlc": {
          "open": 12.96,
          "high": 13.13,
          "low": 12.92,
          "close": 12.96
        }
      }
    },
    "BSE_EQ": {
      "532822": {
        "last_price": 12.96,
        "ohlc": {
          "open": 12.87,
          "high": 13.04,
          "low": 12.83,
          "close": 12.87
        }
      }
    }
  },
  "status": "success"
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| NSE_EQ | int-array | No | List of NSE Equity security IDs as integers (comma-separated, e.g. 1333, 11536) |
| NSE_FNO | int-array | No | List of NSE F&O security IDs as integers (comma-separated, e.g. 49081, 49082) |
| BSE_EQ | int-array | No | List of BSE Equity security IDs as integers (comma-separated) |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | OHLC data |


### Example Request

```bash
curl --request POST \
  --url https://api.dhan.co/v2/marketfeed/ohlc \
  --header 'Accept: application/json' \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --header 'client-id: <your-client-id>' \
  --data '{
    "NSE_EQ": [11536]
  }'
```
