# Set IP

`POST` https://api.dhan.co/v2/ip/setIP

Set a primary or secondary static IP for the account.

Use this endpoint to setup the primary or secondary static IP for the account. Once an IP is setup, it cannot be modified for the next 7 days.


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
curl --request POST \
  --url https://api.dhan.co/v2/ip/setIP \
  --header 'Content-Type: application/json' \
  --header 'access-token: {Access Token}' \
  --data '{
  "dhanClientId": "<your-client-id>",
  "ip": "<your-static-ip>",
  "ipFlag": "PRIMARY"
  }'
```
