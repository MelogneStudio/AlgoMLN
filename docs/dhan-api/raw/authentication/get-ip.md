# Get IP

`GET` https://api.dhan.co/v2/ip/getIP

Get the currently configured primary and secondary IPs and their next modification dates.

Use this endpoint to fetch the currently configured primary and secondary IPs along with the dates on which each can next be modified.


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| modifyDateSecondary | string | No | Date from which the secondary IP can be modified |
| secondaryIP | string | No | Currently set secondary static IP |
| modifyDatePrimary | string | No | Date from which the primary IP can be modified |
| primaryIP | string | No | Currently set primary static IP |


### Example Request

```bash
curl --request GET \
  --url https://api.dhan.co/v2/ip/getIP \
  --header 'access-token: {Access Token}'
```
