# Get Full Quote

`POST` https://api.dhan.co/v2/marketfeed/quote

Get full market quote including market depth (5 levels).

Retrieve full details including market depth, OHLC data, Open Interest and Volume along with LTP for specified instruments.

> **Info:** You can fetch upto 1000 instruments in single API request with rate limit of 1 request per second.

**Before you start with Get Full Quote API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Add security IDs under each exchange segment. Find them in the [instrument list](/api/v2/instruments).


## Sample request for Get Full Quote API

Full quote for Vodafone Idea on NSE (`14366`), its December futures contract on NSE F&O (`58992`) and its BSE equity (`532822`). Replace the client ID and access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/marketfeed/quote \
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


## Sample response for Get Full Quote API

Values are illustrative. Market depth has 5 levels on each side; two are shown here.

```json title="200 OK"
{
  "data": {
    "NSE_EQ": {
      "14366": {
        "average_price": 12.95,
        "buy_quantity": 41250000,
        "sell_quantity": 38760000,
        "last_price": 12.97,
        "last_quantity": 500,
        "last_trade_time": "13/11/2026 15:29:59",
        "lower_circuit_limit": 11.67,
        "upper_circuit_limit": 14.26,
        "net_change": 0.09,
        "volume": 312450000,
        "oi": 0,
        "ohlc": {
          "open": 12.88,
          "high": 13.05,
          "low": 12.84,
          "close": 12.88
        },
        "depth": {
          "buy": [
            {
              "quantity": 125000,
              "price": 12.96,
              "orders": 42
            },
            {
              "quantity": 310000,
              "price": 12.95,
              "orders": 88
            }
          ],
          "sell": [
            {
              "quantity": 98000,
              "price": 12.97,
              "orders": 31
            },
            {
              "quantity": 260000,
              "price": 12.98,
              "orders": 74
            }
          ]
        }
      }
    },
    "NSE_FNO": {
      "58992": {
        "average_price": 13.03,
        "buy_quantity": 41250000,
        "sell_quantity": 38760000,
        "last_price": 13.05,
        "last_quantity": 500,
        "last_trade_time": "13/11/2026 15:29:59",
        "lower_circuit_limit": 11.75,
        "upper_circuit_limit": 14.34,
        "net_change": 0.09,
        "volume": 312450000,
        "oi": 98456250,
        "ohlc": {
          "open": 12.96,
          "high": 13.13,
          "low": 12.92,
          "close": 12.96
        },
        "depth": {
          "buy": [
            {
              "quantity": 125000,
              "price": 13.04,
              "orders": 42
            },
            {
              "quantity": 310000,
              "price": 13.03,
              "orders": 88
            }
          ],
          "sell": [
            {
              "quantity": 98000,
              "price": 13.05,
              "orders": 31
            },
            {
              "quantity": 260000,
              "price": 13.06,
              "orders": 74
            }
          ]
        }
      }
    },
    "BSE_EQ": {
      "532822": {
        "average_price": 12.94,
        "buy_quantity": 41250000,
        "sell_quantity": 38760000,
        "last_price": 12.96,
        "last_quantity": 500,
        "last_trade_time": "13/11/2026 15:29:59",
        "lower_circuit_limit": 11.66,
        "upper_circuit_limit": 14.25,
        "net_change": 0.09,
        "volume": 312450000,
        "oi": 0,
        "ohlc": {
          "open": 12.87,
          "high": 13.04,
          "low": 12.83,
          "close": 12.87
        },
        "depth": {
          "buy": [
            {
              "quantity": 125000,
              "price": 12.95,
              "orders": 42
            },
            {
              "quantity": 310000,
              "price": 12.94,
              "orders": 88
            }
          ],
          "sell": [
            {
              "quantity": 98000,
              "price": 12.96,
              "orders": 31
            },
            {
              "quantity": 260000,
              "price": 12.97,
              "orders": 74
            }
          ]
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
| 200 | Full quote with depth |


### Example Request

```bash
curl --request POST \
  --url https://api.dhan.co/v2/marketfeed/quote \
  --header 'Accept: application/json' \
  --header 'Content-Type: application/json' \
  --header 'access-token: <your-access-token>' \
  --header 'client-id: <your-client-id>' \
  --data '{
    "NSE_EQ": [11536]
  }'
```
