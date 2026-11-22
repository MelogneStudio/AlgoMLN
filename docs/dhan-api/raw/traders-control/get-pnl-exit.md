# Get P&L Based Exit

`GET` https://api.dhan.co/v2/pnlExit

Fetch the currently active P&L based exit configuration.

**Before you start with Get P&L Based Exit API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Paste it into the `access-token` field below.

## Sample request for Get P&L Based Exit API

Fetch the active P&L exit. Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/pnlExit \
  --header 'access-token: <your-access-token>'
```


| Parameter | Data Type | Description |
|-----------|-----------|-------------|
| pnlExitStatus | string | Current status of the P&L exit operation: `ACTIVE` `INACTIVE` |
| profit | float | User-defined target profit amount for the P&L exit |
| loss | float | User-defined target loss amount for the P&L exit |
| enableKillSwitch | boolean | Indicates if the kill switch is enabled for this P&L exit |
| productType | string | Product types applicable: `INTRADAY` `DELIVERY` |

## Sample response for Get P&L Based Exit API

```json title="200 OK"
{
  "pnlExitStatus": "ACTIVE",
  "profit": 1500.0,
  "loss": 500.0,
  "enableKillSwitch": false,
  "productType": [
    "INTRADAY"
  ]
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Status Codes

| Code | Description |
|------|-------------|
| 200 | Current P&L exit config |
| 401 | Unauthorized |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/pnlExit \
  -H "access-token: <your-access-token>"
```
