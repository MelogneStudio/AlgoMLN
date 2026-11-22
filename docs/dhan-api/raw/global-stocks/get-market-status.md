# Get Market Status

`GET` https://api.dhan.co/v2/globalstocks/marketstatus

Retrieve the current trading status and session timing for the US stock market.

Whether the US stock market is open right now, the session open and close times, and whether it is a holiday.

**Before you start with Get Market Status API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Paste it into the `access-token` field below.

## Sample request for Get Market Status API

Fetch the US market status. Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/globalstocks/marketstatus \
  --header 'access-token: <your-access-token>'
```


## Sample response for Get Market Status API

Values are illustrative.

```json title="200 OK"
{
  "marketCloseTime": "16:00",
  "marketOpenTime": "09:30",
  "holidayFlag": false,
  "status": "OPEN"
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| marketCloseTime | string | No | Market close time |
| marketOpenTime | string | No | Market open time |
| holidayFlag | boolean | No | Holiday flag |
| status | string | No | Market status - open/closed |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | market status |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/globalstocks/marketstatus \
  -H "access-token: <your-access-token>"
```
