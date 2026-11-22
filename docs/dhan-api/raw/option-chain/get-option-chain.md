# Get Option Chain

`POST` https://api.dhan.co/v2/optionchain

Retrieve real-time Option Chain across exchanges for all underlying. You can fetch OI, Greeks, Volume, LTP, best bid/ask and IV across all strikes.

Retrieve real-time Option Chain across exchanges for all underlying. You can fetch Open Interest (OI), Greeks, Volume, Last Traded Price, Best Bid/Ask and Implied Volatility (IV) across all strikes for any underlying.

**Before you start with Get Option Chain API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Get a valid expiry from [Expiry List](/api/v2/option-chain/get-expiry-list) first. One unique request is allowed every 3 seconds.

## Sample request for Get Option Chain API

NIFTY (`13`) options expiring on 17 November 2026. Use a current expiry from Expiry List. Replace the client ID and access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/optionchain \
  --header 'Content-Type: application/json' \
  --header 'client-id: <your-client-id>' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "UnderlyingScrip": 13,
    "UnderlyingSeg": "IDX_I",
    "Expiry": "2026-11-17"
  }'
```


## Sample response for Get Option Chain API

Values are illustrative. `oc` has one entry per strike; one strike is shown here.

```json title="200 OK"
{
  "data": {
    "last_price": 25812.4,
    "oc": {
      "25800.000000": {
        "ce": {
          "average_price": 138.6,
          "greeks": {
            "delta": 0.52,
            "theta": -14.8,
            "gamma": 0.0011,
            "vega": 12.6
          },
          "implied_volatility": 13.7,
          "last_price": 140.2,
          "oi": 6234500,
          "previous_close_price": 151.3,
          "previous_oi": 5987000,
          "previous_volume": 98234500,
          "security_id": 42528,
          "top_ask_price": 140.3,
          "top_ask_quantity": 750,
          "top_bid_price": 140.1,
          "top_bid_quantity": 1125,
          "volume": 112345000
        },
        "pe": {
          "average_price": 126.4,
          "greeks": {
            "delta": -0.48,
            "theta": -12.9,
            "gamma": 0.0011,
            "vega": 12.6
          },
          "implied_volatility": 13.9,
          "last_price": 124.8,
          "oi": 5876250,
          "previous_close_price": 118.2,
          "previous_oi": 5612000,
          "previous_volume": 87456000,
          "security_id": 42529,
          "top_ask_price": 124.9,
          "top_ask_quantity": 600,
          "top_bid_price": 124.7,
          "top_bid_quantity": 900,
          "volume": 104567000
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
| UnderlyingScrip | int | Yes | Security ID of underlying instrument (see Annexure) |
| UnderlyingSeg | string | No | Exchange and segment of underlying (see Annexure) |
| Expiry | string | No | Expiry date of option in YYYY-MM-DD (active expiries can be fetched from Expiry List API) |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Option chain data |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/optionchain \
  -H "access-token: <your-access-token>" \
  -H "client-id: <your-client-id>" \
  -H "Content-Type: application/json"
```
