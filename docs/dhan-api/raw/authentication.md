# Authentication

Authentication methods for DhanHQ API — access tokens, API keys, partner login, TOTP, Static IP and more

DhanHQ APIs require authentication based on an access token which needs to be passed with every request. There are various methods to generate this access token depending on user type and the purpose of usage.

**Two categories of users:**

- **Individual** — Users who have a Dhan account and want to build their own algorithm or trading system on top of DhanHQ APIs.
- **Partners** — Platforms that want to build on top of DhanHQ APIs and serve it to their users. This can be algo platforms, fintechs, banks, PMS, and others.

## Eligibility

All Dhan users get access to **Trading APIs for free**. This means you can place and manage orders, positions, funds and all other transactions without paying any extra charges. For Data APIs, there are additional charges mentioned on the platform.

If you are a partner who wants to get integrated and build on top of DhanHQ APIs, you can reach out by filling the form on the [DhanHQ website](https://dhanhq.co/trading-apis).

---

## Authentication APIs

Use the child pages under this section for the endpoint-style pages with request forms and interactive API explorer.

### Access Token

- [Generate Token](/api/v2/authentication/generate-token)
- [Renew Token](/api/v2/authentication/renew-token)

### API Key & Secret

- [Generate Consent](/api/v2/authentication/api-key-generate-consent)
- [Browser Login](/api/v2/authentication/api-key-browser-login)
- [Consume Consent](/api/v2/authentication/api-key-consume-consent)

### Partners

- [Generate Consent](/api/v2/authentication/partner-generate-consent)
- [Browser Login](/api/v2/authentication/partner-browser-login)
- [Consume Consent](/api/v2/authentication/partner-consume-consent)

### Setup Static IP

- [Set IP](/api/v2/authentication/set-ip)
- [Modify IP](/api/v2/authentication/modify-ip)
- [Get IP](/api/v2/authentication/get-ip)

---

## Access for Individual Traders

As an individual trader, there are two methods to generate an access token:

- Directly generate access token from Dhan Web
- Use API key based authentication method

### Access Token

Individual traders can directly get their Access Token from [web.dhan.co](https://web.dhan.co). Here's how:

1. Login to [web.dhan.co](https://web.dhan.co)
2. Click on **Profile** and navigate to **Trading & Data APIs**
3. Click **Generate Access Token / API Key** to generate an **Access Token** valid for 24 hours
4. Optionally enter a Postback URL while generating the token, to get order updates as [Postback](/api/v2/postback)

For API-based token flows, use:

- [Generate Token](/api/v2/authentication/generate-token)
- [Renew Token](/api/v2/authentication/renew-token)

### API Key & Secret

Individuals can also login with an OAuth-based flow. All Dhan users can generate an individual API key and secret.

**To generate API key and secret:**

1. Login to [web.dhan.co](https://web.dhan.co)
2. Click on **Profile** and navigate to **Trading & Data APIs**
3. Click **Generate Access Token / API Key**, then toggle to **API key** and enter your app name
4. Enter **App name**, **Redirect URL** and **Postback URL** as needed

> **note:** API Key & Secret are valid for 12 months from the date of generation.

After getting the API key and secret, the full flow is:

1. [Generate Consent](/api/v2/authentication/api-key-generate-consent)
2. [Browser Login](/api/v2/authentication/api-key-browser-login)
3. [Consume Consent](/api/v2/authentication/api-key-consume-consent)

#### Step 2: Browser Based Login

This endpoint needs to be opened directly on a browser. On this step, the user needs to enter their Dhan credentials, validate with 2FA like OTP, pin, or password. If the login is successful, the user is redirected to the URL provided while generating the API key. Along with the redirect, Dhan sends a `tokenId` which is used in the consume-consent step.

> **note:** This ends with a `302` redirect on the browser. You can consume the `tokenId` from the path parameter directly.

**Request URL**

```text
https://auth.dhan.co/login/consentApp-login?consentAppId={consentAppId}
```

**Path Parameter**

- `consentAppId` — Temporary session ID created in the generate-consent stage

**Response**

```text
{redirect_URL}/?tokenId={Token ID for user}
```

---

## For Partners

Once a partner receives `partner_id` and `partner_secret`, they can use this authentication mechanism for their users.

This login method is a three-step flow for platforms where the user logs into their Dhan account from the third-party product itself.

1. [Generate Consent](/api/v2/authentication/partner-generate-consent)
2. [Browser Login for User](/api/v2/authentication/partner-browser-login)
3. [Consume Consent](/api/v2/authentication/partner-consume-consent)

#### Step 2: Dhan Login on Browser for User

This endpoint needs to be opened directly on a browser tab for browser-based applications or on a webview for mobile apps. On this step, the end user enters their Dhan credentials, validates with 2FA, and is redirected with a `tokenId` for the final consume-consent step.

> **note:** This ends with a `302` redirect on the browser. You can consume the `tokenId` from the path parameter directly.

**Request URL**

```text
https://auth.dhan.co/consent-login?consentId={consentId}
```

**Path Parameter**

- `consentId` — Temporary session ID created in the partner generate-consent stage

**Response**

```text
{redirect_URL}/?tokenId={Token ID for user}
```

---

## Setup Static IP

Static IP whitelisting is mandatory as per the new SEBI and exchange guidelines.

In line with this, you can use the API pages below to set Static IP for your account, or use Dhan Web to set it up manually.

- [Set IP](/api/v2/authentication/set-ip)
- [Modify IP](/api/v2/authentication/modify-ip)
- [Get IP](/api/v2/authentication/get-ip)

Do note that each individual needs to have a unique static IP. Once an IP is whitelisted, it cannot be edited for the next 7 days or as recommended by the exchange.

Do note that Static IP is only required while using Order Placement APIs including Orders and Super Order. While fetching order details or trade details, no such IP whitelisting is required.

> **info:** A static IP is a fixed, permanent internet address for your device or server. Unlike the default IP on home Wi-Fi, a static IP does not change. To use one, you need to request and purchase it separately from your Internet Service Provider (ISP).

---

## Setup TOTP

As an API user, you can setup TOTP to simplify authentication for API-only flows, as an alternative to OTP received on email or mobile number.

### What is TOTP?

Time-based One-Time Password (TOTP) is a 6-digit code generated from a shared secret and current time (RFC 6238). Once you enable TOTP for your account, you’ll receive a secret (via QR/code) that your server can use to generate a fresh code every 30 seconds.

### How to Set Up TOTP

1. Go to Dhan Web → DhanHQ Trading APIs section
2. Select **Setup TOTP**
3. Confirm with OTP on mobile/email
4. Scan the QR via an Authenticator app or enter the code shown
5. Confirm by entering the first TOTP

Once this is setup, you will by default see TOTP as an option while logging into partner platforms or inside the API key based authentication mode.

---

## User Profile

User Profile API can be used to check validity of access token and account setup. It is a simple `GET` request and can be a good first validation call during integration.

```bash
curl --location 'https://api.dhan.co/v2/profile' \
  --header 'access-token: {JWT}'
```
