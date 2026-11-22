# Orders

Global Stocks Orders: buy and sell NYSE and NASDAQ stocks from India, and modify, cancel and track US orders.

The Global Stocks order management API lets you place a new order, modify or cancel a pending order, and retrieve the order book.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/globalstocks/orders`](/api/v2/global-stocks/place-order) | Place a buy or sell order for a US stock using the selected order type, quantity, and price details. |
| PUT | [`/globalstocks/orders/{order-id}`](/api/v2/global-stocks/modify-order) | Modify an existing pending US stock order by submitting updated order details for the given order ID. |
| DELETE | [`/globalstocks/orders/{order-id}`](/api/v2/global-stocks/cancel-order) | Cancel an existing pending US stock order using its order ID. |
| GET | [`/globalstocks/orders/{order-id}`](/api/v2/global-stocks/get-order-by-id) | Retrieve the latest order details and status for a specific US stock order using its order ID. |
| GET | [`/globalstocks/orders`](/api/v2/global-stocks/get-order-book) | Retrieve the current trading day's US stock order book for the authenticated user. |
