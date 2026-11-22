# Generate Consent

`POST` https://auth.dhan.co/partner/generate-consent

Generate a partner consent session for user login.

This validates the partner credentials and creates the consent session required for the browser-login step.

## Sample request for Generate Consent API

Replace `partner_id` and `partner_secret` with the credentials provided by Dhan.

```bash title="cURL"
curl --request POST \
  --url https://auth.dhan.co/partner/generate-consent \
  --header 'Accept: application/json' \
  --header 'partner_id: <your-partner-id>' \
  --header 'partner_secret: <your-partner-secret>'
```


## Sample response for Generate Consent API

```json title="200 OK"
{
  "consentId": "ab5aaab6-38cb-41fc-a074-c816e2f9a3e0",
  "consentStatus": "GENERATED"
}
```


### Header Parameters

| Header | Type | Required | Description |
|--------|------|----------|-------------|
| partner_id | string | Yes | Partner ID provided by Dhan |
| partner_secret | string | Yes | Partner secret provided by Dhan |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| consentId | string | No | Temporary session ID on partner level, used in the browser login step |
| consentStatus | string | No | Status of the partner consent request |


### Example Request

```bash
curl --location 'https://auth.dhan.co/partner/generate-consent' \
  --header 'partner_id: {Partner ID}' \
  --header 'partner_secret: {Partner Secret}'
```
