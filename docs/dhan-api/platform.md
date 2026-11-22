# Platform, authentication and limits

Live sources: [authentication](https://docs.dhanhq.co/api/v2/authentication), [rate limits](https://docs.dhanhq.co/api/v2/rate-limits), [access token](https://docs.dhanhq.co/api/v2/authentication/access-token), [static IP](https://docs.dhanhq.co/api/v2/authentication/static-ip).

## Eligibility and credentials

- Trading APIs are available to Dhan users without a separate API charge; the live docs say data access may require a subscription.
- An individual may generate a 24-hour JWT from Dhan Web, or use the API-key OAuth consent flow. API key and secret expire after 12 months.
- Every normal REST call supplies `access-token: <JWT>`.
- API key flow: `POST https://auth.dhan.co/app/generate-consent` → browser login at `GET https://auth.dhan.co/login/consentApp-login?consentAppId={consentAppId}` → `POST https://auth.dhan.co/app/consumeApp-consent`. The redirect delivers `tokenId`.
- Partner flow uses the partner consent equivalent and browser URL `https://auth.dhan.co/consent-login?consentId={consentId}`.
- Direct-token endpoints: `POST https://auth.dhan.co/app/generateAccessToken` (client ID, PIN, TOTP) and `GET /RenewToken` (renew active token another 24 hours).
- Profile validation: `GET https://api.dhan.co/v2/profile`, authenticated with the access token.

## Static IP requirement

Static-IP whitelisting is mandatory for order placement, modification and cancellation, including Super Order. Reading orders/trades does not need it. Each individual uses a unique IP and an IP cannot be edited for seven days after whitelisting (or longer if exchange guidance requires it).

| Method | Endpoint | Inputs / result |
| --- | --- | --- |
| POST | `/ip/setIP` | primary or secondary static IP |
| PUT | `/ip/modifyIP` | primary or secondary static IP |
| GET | `/ip/getIP` | configured IPs and modification dates |

## Rate limits

| Family | Rate | Per-request cap | Daily limit |
| --- | ---: | ---: | ---: |
| Orders | 10 req/s | — | 7,000 |
| Data APIs | 5 req/s | — | 100,000 |
| Market quote | 1 req/s | 1,000 instruments | — |
| Option chain | 1 request / 3s, plus 10 unique scrips / 8s | 1 underlying/expiry | — |
| Non-trading | 20 req/s | — | unlimited |

On rate limiting, expect `RATE_LIMIT_ERROR` / `RL001`. Batch multi-instrument quotes, cache data, retry with exponential backoff, and pace order submission.

## Important general enums

Exchange segments: `IDX_I`, `NSE_EQ`, `NSE_FNO`, `BSE_EQ`, `MCX_COMM`, `BSE_FNO`. Product types: `CNC`, `INTRADAY`, `MARGIN`, `MTF`. Standard order types: `LIMIT`, `MARKET`, `STOP_LOSS`, `STOP_LOSS_MARKET`; validity: `DAY`, `IOC`.
