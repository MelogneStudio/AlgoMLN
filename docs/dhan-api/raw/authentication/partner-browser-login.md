# Dhan Login on Browser for User

`GET` https://auth.dhan.co/consent-login

Open this URL in a browser tab or webview so the end user can log in to Dhan and authorise the partner app.

This endpoint needs to be opened directly on a tab for browser based applications or on the webview for mobile apps. On this step, the end user needs to enter their Dhan credentials and validate with 2FA like OTP, pin or password. If the login is successful, the user is redirected to the URL provided to Dhan. Along with the redirect, Dhan also sends a `tokenId` which needs to be used in [Consume Consent](/api/v2/authentication/partner-consume-consent).

> **note:** This ends with a `302` redirect on the browser. You can consume the `tokenId` from the path parameter directly.

## Sample request for Dhan Login on Browser for User API

Open this URL in a browser tab or webview, with the `consentId` from Generate Consent.

```text title="URL"
https://auth.dhan.co/consent-login?consentId=ab5aaab6-38cb-41fc-a074-c816e2f9a3e0
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
| consentId | string | Yes | Temporary session ID created in the Generate Consent stage |


### Response Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| tokenId | string | No | Token ID to be used to generate the access token |


### Example Request

```bash
# Open this URL directly in a browser tab or webview
https://auth.dhan.co/consent-login?consentId={consentId}
```
