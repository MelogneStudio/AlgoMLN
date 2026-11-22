# Orders

Orders

The order management API lets you place a new order, cancel or modify the pending order, retrieve the order status, trade status, order book & tradebook.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/orders`](/api/v2/orders/place-order) | Place a new order |
| PUT | [`/orders/{order-id}`](/api/v2/orders/modify-order) | Modify a pending order |
| DELETE | [`/orders/{order-id}`](/api/v2/orders/cancel-order) | Cancel a pending order |
| POST | [`/orders/slicing`](/api/v2/orders/slice-order) | Slice order into multiple legs over freeze limit |
| GET | [`/orders`](/api/v2/orders/get-orders) | Retrieve the list of all orders for the day |
| GET | [`/orders/{order-id}`](/api/v2/orders/get-order-by-id) | Retrieve the status of an order |
| GET | [`/orders/external/{correlation-id}`](/api/v2/orders/get-order-by-correlation-id) | Retrieve the status of an order by correlation id |
| GET | [`/trades`](/api/v2/orders/get-trades) | Retrieve the list of all trades for the day |
| GET | [`/trades/{order-id}`](/api/v2/orders/get-trades-by-order-id) | Retrieve the details of trade by an order id |

> **note:** Order Placement, Modification and Cancellation APIs requires Static IP whitelisting - [here](/api/v2/authentication#setup-static-ip)
