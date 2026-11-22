# Modify IP

`PUT` https://api.dhan.co/v2/ip/modifyIP

Modify a primary or secondary static IP when the modification window is available.

Use this endpoint to modify the configured primary or secondary static IP. This can only be used when the account is within the allowed modification period.


### Request Body Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| dhanClientId | string | Yes | User specific identification |
| ip | string | Yes | Static IP address in IPv4 or IPv6 format |
| ipFlag | enum | Yes | Flag to set the IP as primary or secondary Values: `PRIMARY`, `SECONDARY`. |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| message | string | No | API response message |
| status | string | No | API response status |


### Example Request

```bash
curl --request PUT \
  --url https://api.dhan.co/v2/ip/modifyIP \
  --header 'Content-Type: application/json' \
  --header 'access-token: {Access Token}' \
  --data '{
  "dhanClientId": "<your-client-id>",
  "ip": "<your-static-ip>",
  "ipFlag": "PRIMARY"
  }'
```
