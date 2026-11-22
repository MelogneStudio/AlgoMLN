# Funds & Margin

Global Stocks Funds & Margin: US trading balance, margin calculator and order charges estimator.

The Global Stocks funds and margin APIs let you calculate order estimates, check margin requirements, and retrieve fund limits.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/globalstocks/transEstimate`](/api/v2/global-stocks/order-estimator) | Estimate the applicable charges for a US stock order before submitting it. |
| GET | [`/globalstocks/fundlimit`](/api/v2/global-stocks/get-fund-limit) | Retrieve the authenticated user's fund limit details for US stock trading. |
| POST | [`/globalstocks/margincalculator`](/api/v2/global-stocks/margin-calculator) | Calculate the margin required for a US stock order before submitting it. |
