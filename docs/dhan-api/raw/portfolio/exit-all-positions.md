# Exit All Positions

`DELETE` https://api.dhan.co/v2/positions

Exit all open positions at market price.

Exit all active positions and cancel all open orders for the current trading day.

**Before you start with Exit All Positions API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Whitelist your [static IP](/api/v2/authentication/set-ip). Orders only go through from it.
3. This exits **every** open position and cancels all open orders right away.

> **NOTICE:** Order Placement, Modification and Cancellation APIs requires Static IP whitelisting - [refer here](/api/v2/authentication#setup-static-ip)

## Sample request for Exit All Positions API

Exit all positions and cancel all open orders. There is no request body. Replace the access token with your own.

```bash title="cURL"
curl --request DELETE \
  --url https://api.dhan.co/v2/positions \
  --header 'access-token: <your-access-token>'
```


No response body. On successful completion of request, `202 Accepted` response status code will appear.

## Sample response for Exit All Positions API

`202 Accepted`. There is no response body.

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Status Codes

| Code | Description |
|------|-------------|
| 200 | All positions exited |


### Example Request

```bash
curl -X DELETE https://api.dhan.co/v2/positions \
  -H "access-token: <your-access-token>"
```
