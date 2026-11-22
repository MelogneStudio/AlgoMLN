# DhanHQ API v2 feature matrix

Live documentation entry points: [feature matrix](https://docs.dhanhq.co/api/v2/feature-matrix), [sandbox](https://docs.dhanhq.co/api/v2/sandbox), [releases](https://docs.dhanhq.co/api/v2/releases).

## Functional coverage map

| Capability | REST | WebSocket / callback | Requirement | Local details |
| --- | --- | --- | --- | --- |
| Indian orders | yes | postback + live updates | JWT; static IP for create/modify/cancel | [trading.md](trading.md), [events.md](events.md) |
| Super orders | yes | order events | JWT + static IP | [trading.md](trading.md) |
| Conditional / multi orders | yes | optional postback | equities and indices only for conditional | [trading.md](trading.md), [reference.md](reference.md) |
| Holdings / positions / EDIS / statements | yes | — | JWT | [trading.md](trading.md) |
| Fund/margin/risk controls | yes | — | JWT | [trading.md](trading.md) |
| Snapshot market data | yes | — | data subscription; quote rate limits | [market-data.md](market-data.md) |
| Historical / rolling options | yes | — | data subscription | [market-data.md](market-data.md) |
| Option chain | yes | — | data subscription; 3-second and unique-scrip caps | [market-data.md](market-data.md) |
| Live feed / 20 / 200 depth / CAS | — | binary websocket | JWT; Pro plan for 200/CAS | [market-data.md](market-data.md) |
| Global Stocks | yes | live websocket | global instrument master + account access | [global-stocks.md](global-stocks.md) |

## Implementation checklist

1. Fetch and refresh the appropriate instrument master.
2. Create/renew an access token and validate it with `/profile`.
3. Subscribe to data APIs if consuming quotes/candles/options.
4. Configure static IP before any order state change.
5. Use `correlationId` on new orders and persist returned `orderId`.
6. Consume Postback or Live Order Update for authoritative asynchronous state.
7. Apply client-side rate limiters per [platform.md](platform.md).

## Documentation freshness

All endpoint claims in this directory came from rendered pages on the official live documentation site. The live site occasionally resolves deep-link pages to its marketing shell; where that happened, this map retains only calls and schemas actually present in pages successfully rendered during this pass, rather than inventing undocumented fields.
