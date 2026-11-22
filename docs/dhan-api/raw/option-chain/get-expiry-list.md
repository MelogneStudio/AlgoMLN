# Get Expiry List

`POST` https://api.dhan.co/v2/optionchain/expirylist

Get available expiry dates for an underlying.

Retrieve dates of all expiries of any underlying, for which Options Instruments are active.

**Before you start with Get Expiry List API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Data APIs need an active Data API subscription on your account.
3. Enter the underlying's security ID and segment, for example `13` and `IDX_I` for NIFTY.

## Sample request for Get Expiry List API

Expiry dates for NIFTY (`13`). Replace the client ID and access token with your own.

```bash title="cURL"
curl --request POST \
  --url https://api.dhan.co/v2/optionchain/expirylist \
  --header 'Content-Type: application/json' \
  --header 'client-id: <your-client-id>' \
  --header 'access-token: <your-access-token>' \
  --data '{
    "UnderlyingScrip": 13,
    "UnderlyingSeg": "IDX_I"
  }'
```


## Sample response for Get Expiry List API

Values are illustrative.

```json title="200 OK"
{
  "data": [
    "2026-11-17",
    "2026-11-24",
    "2026-12-01",
    "2026-12-08",
    "2026-12-29"
  ],
  "status": "success"
}
```

Check the [Annexure](/api/v2/annexure#data-api-error) for error codes.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| UnderlyingScrip | int | Yes | Security ID of underlying instrument (see Annexure) |
| UnderlyingSeg | string | No | Exchange and segment of underlying (see Annexure) |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| data[] | array | No | All expiry dates of underlying in YYYY-MM-DD |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | List of expiry dates |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/optionchain/expirylist \
  -H "access-token: <your-access-token>" \
  -H "client-id: <your-client-id>" \
  -H "Content-Type: application/json"
```
