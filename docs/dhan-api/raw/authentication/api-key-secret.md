# API Key & Secret

Generate an access token with an API key and secret, through consent and browser login.

Individuals can also login with an OAuth-based flow. All Dhan users can generate an individual API key and secret.

**To generate API key and secret:**

1. Login to [web.dhan.co](https://web.dhan.co)
2. Click on **Profile** and navigate to **Trading & Data APIs**
3. Click **Generate Access Token / API Key**, then toggle to **API key** and enter your app name
4. Enter **App name**, **Redirect URL** and **Postback URL** as needed

> **note:** API Key & Secret are valid for 12 months from the date of generation.

After getting the API key and secret, the full flow is:

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`https://auth.dhan.co/app/generate-consent`](/api/v2/authentication/api-key-generate-consent) | Validate the app credentials and create a consent session |
| GET | [`https://auth.dhan.co/login/consentApp-login`](/api/v2/authentication/api-key-browser-login) | Log in on the browser and get a token ID on the redirect URL |
| POST | [`https://auth.dhan.co/app/consumeApp-consent`](/api/v2/authentication/api-key-consume-consent) | Generate the access token from the token ID |
