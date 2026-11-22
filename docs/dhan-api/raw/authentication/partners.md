# Partners

Partner login flow to generate access tokens for Dhan users.

Once a partner receives `partner_id` and `partner_secret`, they can use this authentication mechanism for their users.

This login method is a three-step flow for platforms where the user logs into their Dhan account from the third-party product itself.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`https://auth.dhan.co/partner/generate-consent`](/api/v2/authentication/partner-generate-consent) | Validate the partner credentials and create a consent session |
| GET | [`https://auth.dhan.co/consent-login`](/api/v2/authentication/partner-browser-login) | User logs in on the browser and gets a token ID on the redirect URL |
| POST | [`https://auth.dhan.co/partner/consume-consent`](/api/v2/authentication/partner-consume-consent) | Generate the user access token from the token ID |
