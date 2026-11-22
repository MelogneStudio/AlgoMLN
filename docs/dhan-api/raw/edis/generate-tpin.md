# Generate T-PIN

`GET` https://api.dhan.co/v2/edis/tpin

Request a T-PIN on your registered mobile number for eDIS authorization.

Get T-Pin on your registered mobile number using this API.

**Before you start with Generate T-PIN API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. This sends a T-PIN by SMS to your registered mobile number.

## Sample request for Generate T-PIN API

Request a T-PIN. Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/edis/tpin \
  --header 'access-token: <your-access-token>'
```

## Response

`202 Accepted` — the T-PIN is sent to your registered mobile number. This endpoint returns no response body.

## Sample response for Generate T-PIN API

`202 Accepted`. There is no response body; the T-PIN is sent by SMS to your registered mobile number.

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Status Codes

| Code | Description |
|------|-------------|
| 202 | Accepted — T-PIN sent to your registered mobile number. No response body. |
| 401 | Unauthorized |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/edis/tpin \
  -H "access-token: <your-access-token>"
```
