# Super Order

Super Order: Dhan's bracket order API. Place, modify and cancel orders with entry, target, stop-loss and trailing stop loss in one request.

Super orders are built for smart execution of your trades. They are a collection of orders clubbed into single order request, which includes entry leg, target leg and stop loss leg along with the option to add trailing stop loss.

This particular set of APIs can be used to create, modify and cancel super orders. You can place super orders across all exchanges and segments.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/super/orders`](/api/v2/super-orders/place-super-order) | Create a new super order |
| PUT | [`/super/orders/{order-id}`](/api/v2/super-orders/modify-super-order) | Modify a pending super order |
| DELETE | [`/super/orders/{order-id}/{order-leg}`](/api/v2/super-orders/cancel-super-order-leg) | Cancel a pending super order leg |
| GET | [`/super/orders`](/api/v2/super-orders/get-super-orders) | Retrieve the list of all super orders |
