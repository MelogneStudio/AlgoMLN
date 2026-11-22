# Account events: Postback and Live Order Update

Live sources: [Postback](https://docs.dhanhq.co/api/v2/postback), [Live Order Update](https://docs.dhanhq.co/api/v2/order-update).

## HTTP Postback

Set a Postback URL when generating an access token in Dhan Web. Dhan then POSTs raw JSON whenever a related order changes state, changes after modification, or is partially filled. It applies to all trades originating from that token. Localhost and `127.0.0.1` cannot receive Postback requests.

Payload includes `dhanClientId`, `orderId`, `correlationId`, `orderStatus`, order identity (`transactionType`, `exchangeSegment`, `productType`, `orderType`, `validity`, `tradingSymbol`, `securityId`), quantities/prices, AMO flag, BO values, `legName`, timestamps, derivative data, any OMS error, `filled_qty`, and `algoId`.

For apps that need events for all platform-originated orders, use the Partner Login module.

## Live Order Update WebSocket

Endpoint: `wss://api-order-update.dhan.co`. Frames are JSON and emitted with `Type: "order_alert"`.

Individual authorization:

```json
{"LoginReq":{"MsgCode":42,"ClientId":"<client-id>","Token":"<JWT>"},"UserType":"SELF"}
```

Partner authorization:

```json
{"LoginReq":{"MsgCode":42,"ClientId":"<partner-id>"},"UserType":"PARTNER","Secret":"<partner-secret>"}
```

The payload `Data` identifies exchange/segment/source/security/client/order IDs; product, transaction and order type; all quantities and prices; timing; order status/reason; super-order leg; contract metadata; correlation ID; and algorithm fields. Numeric product codes are `C` CNC, `I` intraday, `M` margin, `F` MTF, `V` cover, `B` bracket. Transaction codes are `B` and `S`; order type codes include `LMT`, `MKT`, `SL`, `SLM`.
