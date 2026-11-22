# Sandbox

Test DhanHQ API integrations in the sandbox environment

DhanHQ Sandbox is the testing environment for DhanHQ API integrations. Use it to validate request payloads, authentication handling, response parsing, and order-management workflows before pointing your application to the live API.

Sandbox requests use the sandbox base URL:

```text
https://sandbox.dhan.co/v2
```

Generate a sandbox access token from Dhan Cloud at [cloud.dhanhq.co](https://cloud.dhanhq.co/). Use this sandbox token in the same `access-token` request header used by production APIs.

```bash
curl --request GET \
  --url https://sandbox.dhan.co/v2/orders \
  --header 'access-token: <your-sandbox-access-token>' \
  --header 'Content-Type: application/json'
```

Sandbox and production are separate environments:

- Use `https://sandbox.dhan.co/v2` with a sandbox token for testing.
- Use `https://api.dhan.co/v2` with a production access token for live API calls.
- Both of these environments can be accessed in the documentation for testing and live API calls.
- Do not reuse production tokens for sandbox requests or sandbox tokens for production requests.
- Keep the request method, path, headers, and payload structure the same as the documented endpoint unless the endpoint documentation states otherwise.

Only endpoints marked as sandbox-enabled in this documentation are available in the sandbox environment:

| Method | Endpoint | Path |
|--------|----------|------|
| POST | [Place Order](/api/v2/orders/place-order) | `/orders` |
| PUT | [Modify Order](/api/v2/orders/modify-order) | `/orders/{orderId}` |
| DELETE | [Cancel Order](/api/v2/orders/cancel-order) | `/orders/{order-id}` |
| GET | [Get Order Book](/api/v2/orders/get-orders) | `/orders` |
| GET | [Get Order by ID](/api/v2/orders/get-order-by-id) | `/orders/{order-id}` |
| GET | [Get Order by Correlation ID](/api/v2/orders/get-order-by-correlation-id) | `/orders/external/{correlation-id}` |
| POST | [Slice Order](/api/v2/orders/slice-order) | `/orders/slicing` |
| GET | [Get Trade Book](/api/v2/orders/get-trades) | `/trades` |
| GET | [Get Trades for Order](/api/v2/orders/get-trades-by-order-id) | `/trades/{order-id}` |
| GET | [Get Holdings](/api/v2/portfolio/get-holdings) | `/holdings` |
| GET | [Get Positions](/api/v2/portfolio/get-positions) | `/positions` |
| POST | [Convert Position](/api/v2/portfolio/convert-position) | `/positions/convert` |
| POST | [Manage Kill Switch](/api/v2/traders-control/manage-kill-switch) | `/killswitch` |
| GET | [Generate T-PIN](/api/v2/edis/generate-tpin) | `/edis/tpin` |
| POST | [Generate eDIS Form](/api/v2/edis/generate-edis-form) | `/edis/bulkform` |
| GET | [EDIS Status Inquiry](/api/v2/edis/edis-inquiry) | `/edis/inquire/{isin}` |
| GET | [Get Fund Limits](/api/v2/funds/get-fund-limits) | `/fundlimit` |
| POST | [Calculate Margin](/api/v2/funds/calculate-margin) | `/margincalculator` |
| GET | [Ledger Report](/api/v2/statements/get-ledger-report) | `/ledger` |
| GET | [Trade History](/api/v2/statements/get-trade-history) | `/trades/{from-date}/{to-date}/{page-number}` |
| POST | [Get Intraday Historical Data](/api/v2/historical-data/get-intraday-historical) | `/charts/intraday` |
| POST | [Get Daily Historical Data](/api/v2/historical-data/get-daily-historical) | `/charts/historical` |
