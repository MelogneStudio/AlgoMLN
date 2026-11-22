# EDIS Status Inquiry

`GET` https://api.dhan.co/v2/edis/inquire/{isin}

Check eDIS approval status for a stock by ISIN. Pass ALL to check all holdings.

You can check the status of stock whether it is approved and marked for sell action. Users have to enter the ISIN of the stock. An International Securities Identification Number (ISIN) is a 12-digit alphanumeric code that uniquely identifies a specific security. You can get ISIN of portfolio stocks, in response of holdings API.

Alternatively, you can pass "ALL" instead of ISIN to get eDIS status of all holdings in your portfolio.

**Before you start with EDIS Status Inquiry API**

1. Generate an access token on [web.dhan.co](https://web.dhan.co) under **Profile → Trading & Data APIs**.
2. Enter the stock's ISIN under Path Parameters below.

## Sample request for EDIS Status Inquiry API

Check the eDIS status of Vodafone Idea (ISIN `INE669E01016`). Replace the access token with your own.

```bash title="cURL"
curl --request GET \
  --url https://api.dhan.co/v2/edis/inquire/INE669E01016 \
  --header 'access-token: <your-access-token>'
```


## Sample response for EDIS Status Inquiry API

```json title="200 OK"
{
  "clientId": "1000000132",
  "isin": "INE669E01016",
  "totalQty": "1",
  "aprvdQty": "1",
  "status": "SUCCESS",
  "remark": "eDIS transaction done successfully"
}
```

Check the [Annexure](/api/v2/annexure#trading-api-error) for error codes.


### Path Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| isin | string | Yes | ISIN of portfolio stocks, derived from holdings API |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| clientId | string | No | User specific identification |
| isin | string | No | International Securities Identification Number |
| totalQty | string | No | Total number of shares for given stock |
| aprvdQty | string | No | Number of approved stocks |
| status | enum string | No | Status of the eDIS order Values: `SUCCESS`, `FAILURE`. |
| remark | string | No | Remarks of the order status |


### Status Codes

| Code | Description |
|------|-------------|
| 200 | eDIS status |
| 401 | Unauthorized |


### Example Request

```bash
curl -X GET https://api.dhan.co/v2/edis/inquire/{isin} \
  -H "access-token: <your-access-token>"
```
