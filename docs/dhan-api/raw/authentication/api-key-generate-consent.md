# Generate Consent

`POST` https://auth.dhan.co/app/generate-consent

Generate a consent session for API Key & Secret based login.

This validates the app credentials and creates a consent session that is used in the browser-login step.

> **note:** Users can generate up to 25 `consentAppId` values per day. Each one stays active until `tokenId` is generated, but only one token stays active at a time.

## Sample request for Generate Consent API

Replace the client ID with your own, and `app_id` and `app_secret` with the API Key and API Secret of your app.

```bash title="cURL"
curl --request POST \
  --url 'https://auth.dhan.co/app/generate-consent?client_id=1000000001' \
  --header 'Accept: application/json' \
  --header 'app_id: <your-api-key>' \
  --header 'app_secret: <your-api-secret>'
```


## Sample response for Generate Consent API

```json title="200 OK"
{
  "consentAppId": "8f14e45f-ceea-467a-9575-0c2b5b3b4f21",
  "consentAppStatus": "GENERATED",
  "status": "success"
}
```


### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| client_id | string | Yes | Dhan client ID |


### Header Parameters

| Header | Type | Required | Description |
|--------|------|----------|-------------|
| app_id | string | Yes | API key generated from Dhan |
| app_secret | string | Yes | API secret generated from Dhan |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| consentAppId | string | No | Temporary session ID to be used in the browser login step |
| consentAppStatus | string | No | Status of the consent creation request |
| status | string | No | API response status |


### Example Request

```bash
curl --location --request POST \
  'https://auth.dhan.co/app/generate-consent?client_id={dhanClientId}' \
  --header 'app_id: {API key}' \
  --header 'app_secret: {API secret}'
```
