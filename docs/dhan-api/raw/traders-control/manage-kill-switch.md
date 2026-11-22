# Manage Kill Switch

`POST` https://api.dhan.co/v2/killswitch

Activate or deactivate the kill switch for your account. Disables trading for the current day.

This API lets you activate the kill switch for your account, which will disable trading for current trading day. You can pass header parameter as ACTIVATE or DEACTIVATE to manage Kill Switch settings.

> **Note:** You need to ensure that all your positions are closed and there are no pending orders in your account to be able to activate Kill Switch.

**Before you start with Manage Kill Switch API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Whitelist your [static IP](/api/v2/authentication/set-ip). Orders only go through from it.
3. Choose `ACTIVATE` or `DEACTIVATE` under Query Parameters below. Activating stops all trading on your account for the rest of the day.

> **NOTICE:** Order Placement, Modification and Cancellation APIs requires Static IP whitelisting - [refer here](/api/v2/authentication#setup-static-ip)

## Sample request for Manage Kill Switch API

Activate the kill switch. There is no request body. Replace the access token with your own.

```bash title="cURL"
curl --request POST \
  --url 'https://api.dhan.co/v2/killswitch?killSwitchStatus=ACTIVATE' \
  --header 'access-token: <your-access-token>'
```


## Sample response for Manage Kill Switch API

```json title="200 OK"
{
  "killSwitchStatus": "ACTIVATE"
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| killSwitchStatus | string | No | Kill switch status Values: `ACTIVATE`, `DEACTIVATE`. |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| killSwitchStatus | string | No | Status of Kill Switch Values: `ACTIVATE`, `DEACTIVATE`. |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Kill switch status updated |
| 401 | Unauthorized |


### Example Request

```bash
curl -X POST https://api.dhan.co/v2/killswitch \
  -H "access-token: <your-access-token>" \
  -H "Content-Type: application/json"
```
