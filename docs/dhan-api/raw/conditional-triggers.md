# Conditional and Multi Order

Conditional and Multi Order: price and technical indicator conditional orders, and multi orders with up to 15 orders in one Dhan API call.

The Conditional and Multi Order API is a special set of APIs which lets you place order on the basis of set conditions. These conditions can be based on price or technical indicators or a combination of both. You can set one or multiple orders to be triggered when the condition is met.

When the conditional order is triggered, you will receive a postback update if set up [here](/api/v2/postback).

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/alerts/orders`](/api/v2/conditional-triggers/place-conditional-trigger) | Place Conditional Order |
| POST | [`/alerts/multi/orders`](/api/v2/conditional-triggers/place-multi-order) | Place Multi Order |
| PUT | [`/alerts/orders/{alertId}`](/api/v2/conditional-triggers/modify-conditional-trigger) | Modify Conditional Order |
| DELETE | [`/alerts/orders/{alertId}`](/api/v2/conditional-triggers/delete-conditional-trigger) | Delete Conditional Order |
| GET | [`/alerts/orders/{alertId}`](/api/v2/conditional-triggers/get-conditional-trigger-by-id) | Get Conditional Order by ID |
| GET | [`/alerts/orders`](/api/v2/conditional-triggers/get-all-conditional-triggers) | Get All Conditional Orders |

> **note:** - Conditional Orders are currently supported only for Equities and Indices.
> - You can receive a postback update by providing a Webhook URL ([here](/api/v2/postback)) while generating the Access Token.
