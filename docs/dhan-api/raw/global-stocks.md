# Global Stocks

Global Stocks

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [Place Order](/api/v2/global-stocks/place-order) | Place a buy or sell order for a US stock using the selected order type, quantity, and price details. |
| PUT | [Modify Order](/api/v2/global-stocks/modify-order) | Modify an existing pending US stock order by submitting updated order details for the given order ID. |
| GET | [Get Order by ID](/api/v2/global-stocks/get-order-by-id) | Retrieve the latest order details and status for a specific US stock order using its order ID. |
| DELETE | [Cancel Order](/api/v2/global-stocks/cancel-order) | Cancel an existing pending US stock order using its order ID. |
| GET | [Get Order Book](/api/v2/global-stocks/get-order-book) | Retrieve the current trading day's US stock order book for the authenticated user. |
| POST | [Order Estimator](/api/v2/global-stocks/order-estimator) | Estimate the applicable charges for a US stock order before submitting it. |
| POST | [Margin Calculator](/api/v2/global-stocks/margin-calculator) | Calculate the margin required for a US stock order before submitting it. |
| GET | [Get Past Trades](/api/v2/global-stocks/get-past-trades) | Retrieve the current trading day's executed US stock trades for the authenticated user. |
| GET | [Get Past Trades by Security Id](/api/v2/global-stocks/get-past-trades-by-scrip) | Retrieve executed US stock trades for the authenticated user filtered by a specific security ID. |
| GET | [Get Market Status](/api/v2/global-stocks/get-market-status) | Retrieve the current trading status and session timing for the US stock market. |
| WebSocket | [Global Stocks Live Feed](/api/v2/global-stocks/data-apis/broadcast-websocket) | Subscribe to real-time Trade and OHLC packets for Global Stocks. |
| GET | [Get Holdings](/api/v2/global-stocks/get-holdings) | Retrieve the authenticated user's current US stock holdings and holding-level details. |
| GET | [Get Fund Limit](/api/v2/global-stocks/get-fund-limit) | Retrieve the authenticated user's fund limit details for US stock trading. |
