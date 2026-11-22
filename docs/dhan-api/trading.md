# Trading REST API map

Live sources: [trading overview](https://docs.dhanhq.co/api/v2/trading-apis), [orders](https://docs.dhanhq.co/api/v2/orders), [place order](https://docs.dhanhq.co/api/v2/orders/place-order), [modify order](https://docs.dhanhq.co/api/v2/orders/modify-order).

All state-changing order calls require an active access token and whitelisted static IP. Use the instrument master to select `securityId` and `exchangeSegment`. A successful create acknowledges Dhan acceptance; it is not proof of an exchange fill. Track via order lookup, postback or live order feed.

## Orders and trades

| Method | Endpoint | Purpose | Inputs |
| --- | --- | --- | --- |
| POST | `/orders` | Place order | required `dhanClientId`, `transactionType` (`BUY`/`SELL`), `exchangeSegment`, `productType`, `orderType`, `validity`, `securityId`, `quantity`; optional `correlationId` (max 30, `[a-zA-Z0-9-_]`), `disclosedQuantity`, `price`, `triggerPrice`, `afterMarketOrder`, `amoTime`, `algoId` |
| PUT | `/orders/{orderId}` | Modify pending order | path `orderId`; required body `dhanClientId`, `orderId`, `orderType`, `validity`; optional `quantity`, `price`, `disclosedQuantity`, `triggerPrice`, `legName` |
| DELETE | `/orders/{orderId}` | Cancel pending order | path order ID |
| POST | `/orders/slicing` | Split a large F&O order at freeze limit | place-order fields; docs mark `dhanClientId`, side, segment, product, type, validity, quantity and price required |
| GET | `/orders` | Today’s order book | access token |
| GET | `/orders/{orderId}` | Specific order | path order ID |
| GET | `/orders/external/{correlationId}` | Look up by caller correlation | path correlation ID |
| GET | `/trades` | Today’s trade book | access token |
| GET | `/trades/{orderId}` | Trades belonging to order | path order ID |

Order create/modify response: `orderId`, `orderStatus`. The documented statuses include `TRANSIT`, `PENDING`, `REJECTED`, `CANCELLED`, `PART_TRADED`, `TRADED`, `EXPIRED`, `MODIFIED`, `TRIGGERED`.

## Super orders

Super Order bundles entry, target and stop-loss legs; supports trailing stop loss and all exchanges/segments.

| Method | Endpoint | Purpose |
| --- | --- | --- |
| POST | `/super/orders` | Create entry/target/SL order set |
| PUT | `/super/orders/{orderId}` | Modify a pending super order |
| DELETE | `/super/orders/{orderId}/{orderLeg}` | Cancel a pending leg |
| GET | `/super/orders` | List super orders |

The place example in the live Python docs uses `security_id`, `exchange_segment`, `transaction_type`, `quantity`, `order_type`, `price`, `target_price`, `stop_loss_price`, `trailing_jump`. `legName` values used for modification/cancellation are `ENTRY_LEG`, `TARGET_LEG`, `STOP_LOSS_LEG`.

## Conditional and multi orders

| Method | Endpoint | Purpose |
| --- | --- | --- |
| POST | `/alerts/orders` | Create conditional trigger |
| POST | `/alerts/multi/orders` | Submit multi-order without conditions |
| PUT | `/alerts/orders/{alertId}` | Modify trigger |
| DELETE | `/alerts/orders/{alertId}` | Delete trigger |
| GET | `/alerts/orders/{alertId}` | One trigger |
| GET | `/alerts/orders` | All triggers |

Conditional orders currently support equities and indices only. Conditions may use price, technical indicators, or combinations. See [reference.md](reference.md) for comparison types, indicator IDs, operators and statuses. A configured token Postback URL receives trigger updates.

## Portfolio, risk, funds and records

| Area | Method | Endpoint | Inputs / purpose |
| --- | --- | --- | --- |
| Portfolio | GET | `/holdings` | demat holdings |
| Portfolio | GET | `/positions` | open positions |
| Portfolio | POST | `/positions/convert` | position conversion; Python surface shows `securityId`, `exchangeSegment`, `positionType`, `convertQty`, `fromProductType`, `toProductType` |
| Portfolio | DELETE | `/positions` | exit all positions |
| EDIS | GET | `/edis/tpin` | generate T-PIN |
| EDIS | POST | `/edis/from` | retrieve escaped authorization form; user enters T-PIN |
| EDIS | GET | `/edis/inquire/{isin}` | EDIS authorization status |
| Funds | GET | `/fundlimit` | available funds |
| Funds | POST | `/margincalculator` | one-order margin |
| Funds | POST | `/margincalculator/multi` | multiple-order margin |
| Risk | POST/GET | `/killswitch` | enable/disable or retrieve kill-switch status |
| Risk | POST/GET/DELETE | `/pnlExit` | configure/read/stop P&L auto-exit |
| Statement | GET | `/ledger` | account debit/credit ledger |
| Statement | GET | `/trades/` | historical trade data |
