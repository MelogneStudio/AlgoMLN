# Browser Based Login

`GET` https://auth.dhan.co/login/consentApp-login

Open this URL in a browser so the user can log in to Dhan and authorise the app.

This endpoint needs to be opened directly on a browser. On this step, the user needs to enter their Dhan credentials and validate with 2FA like OTP, pin or password. If the login is successful, the user is redirected to the URL provided while generating the API key. Along with the redirect, Dhan also sends a `tokenId` which needs to be used in [Consume Consent](/api/v2/authentication/api-key-consume-consent).

> **note:** This ends with a `302` redirect on the browser. You can consume the `tokenId` from the path parameter directly.

## Sample request for Browser Based Login API

Open this URL in a browser, with the `consentAppId` from Generate Consent.

```text title="URL"
https://auth.dhan.co/login/consentApp-login?consentAppId=8f14e45f-ceea-467a-9575-0c2b5b3b4f21
```

## Response Structure

```text
{redirect_URL}/?tokenId={Token ID for user}
```


## Sample redirect

After login, the browser opens your redirect URL with `tokenId` added at the end. Copy the `tokenId` value for Consume Consent.

```text title="Redirect URL"
https://your-redirect-url.com/?tokenId=b7e2d9a4-3c1f-4e8b-9a6d-2f5c8e1b7a30
```


### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| consentAppId | string | Yes | Temporary session ID created in the Generate Consent stage |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| tokenId | string | No | Token ID to be used to generate the access token |


### Example Request

```bash
# Open this URL directly in a browser
https://auth.dhan.co/login/consentApp-login?consentAppId={consentAppId}
```
