# Stop P&L Based Exit

`DELETE` https://api.dhan.co/v2/pnlExit

Disable the active P&L based exit configuration.

**Before you start with Stop P&L Based Exit API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Whitelist your [static IP](/api/v2/authentication/set-ip). Orders only go through from it.

> **NOTICE:** Order Placement, Modification and Cancellation APIs requires Static IP whitelisting - [refer here](/api/v2/authentication#setup-static-ip)

## Sample request for Stop P&L Based Exit API

Stop the P&L exit. There is no request body. Replace the access token with your own.

```bash title="cURL"
curl --request DELETE \
  --url https://api.dhan.co/v2/pnlExit \
  --header 'access-token: <your-access-token>'
```


| Parameter | Data Type | Description |
|-----------|-----------|-------------|
| pnlExitStatus | string | P&L based exit status: `ACTIVE` `INACTIVE` |
| message | string | Status of Conditional Trigger |

## Sample response for Stop P&L Based Exit API

```json title="200 OK"
{
  "pnlExitStatus": "INACTIVE",
  "message": "P&L based exit stopped successfully"
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Status Codes

| Code | Description |
|------|-------------|
| 200 | P&L exit stopped |
| 401 | Unauthorized |


### Example Request

```bash
curl -X DELETE https://api.dhan.co/v2/pnlExit \
  -H "access-token: <your-access-token>"
```
