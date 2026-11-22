# Access Token

Generate and renew a DhanHQ access token.

Individual traders can directly get their Access Token from [web.dhan.co](https://web.dhan.co). Here's how:

1. Login to [web.dhan.co](https://web.dhan.co)
2. Click on **Profile** and navigate to **Trading & Data APIs**
3. Click **Generate Access Token / API Key** to generate an **Access Token** valid for 24 hours
4. Optionally enter a Postback URL while generating the token, to get order updates as [Postback](/api/v2/postback)

The access token can also be generated and renewed through the APIs below.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`https://auth.dhan.co/app/generateAccessToken`](/api/v2/authentication/generate-token) | Generate an access token with client ID, PIN and TOTP |
| GET | [`/RenewToken`](/api/v2/authentication/renew-token) | Renew an active token for another 24 hours |
