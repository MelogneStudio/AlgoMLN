# Dhan API Endpoints Used by AlgoMLN

**Base URL:** `https://api.dhan.co/v2`
**Auth:** `access-token: <DHAN_ACCESS_TOKEN>` header on every request
**Caller:** `src/broker/dhan/rest.rs` (`DhanClient`) and `src/broker/symbol_map.rs`

## REST endpoints

| # | Method | Path | Purpose | Caller |
|---|---|---|---|---|
| 1 | POST | `/orders` | Place order (no retry; timed-out requests return `DhanError::OrderStatusUnknown`). Generates `correlationId = "algomln-<uuid>"` for idempotency. | `DhanClient::place_order` |
| 2 | GET | `/positions` | Open positions (filter `net_qty != 0`). Used for realized-loss, `MAX_DAILY_LOSS` enforcement, stale-cache flag, preflight gate, live-status polling. | `DhanClient::get_positions` |
| 3 | POST | `/marketfeed/quote` | LTP/snapshot for one or more symbols. Body is segment-keyed: `{ "NSE_EQ": ["2885"] }`. | `DhanClient::get_quote` |
| 4 | POST | `/charts/intraday` | Intraday candles (1/5/15/25/60 min). Auto-chunked into 89-day windows. Timestamps = Unix seconds → ms. | `DhanClient::get_ohlcv_intraday` |
| 5 | POST | `/charts/historical` | Daily/weekly candles. Body uses date strings (`YYYY-MM-DD`). Timestamps use 1980-01-01 epoch offset. | `DhanClient::get_ohlcv` (non-intraday) |
| 6 | GET | `/funds/limit` | Account funds limit (returns `availablebalance` + collateral / utilized / withdrawable figures). Consumed by `DhanBroker::available_cash` so `PercentCapital` order sizing reflects real buying power, capped at `DEFAULT_AVAILABLE_CASH_CAP` (1 lakh INR) until the first successful refresh. Refreshed every `FUNDS_REFRESH_INTERVAL_SECS` (60 s). | `DhanClient::get_funds_limit` |

## Static data

| # | URL | Purpose | Caller |
|---|---|---|---|
| 7 | GET `https://images.dhan.co/api-data/api-scrip-master-detailed.csv` | NSE scrip master (no auth). Refreshes `<app_data>/sec_id_cache.csv` if older than 7 days. User-Agent: `AlgoMLN/1.0`. | `refresh_symbol_map` |

## Notes

- **Phase 7 guard:** `place_order` rejects non-`NSE_EQ` segments before sending (`rest.rs:475`).
- **Order idempotency:** Every `POST /orders` carries `correlationId: "algomln-<uuid>"` so a retried/duplicated request is deduped by the broker.
- **No retry on `/orders`:** Order placement uses `post_order_no_retry` — timeout returns `DhanError::OrderStatusUnknown { correlation_id }` so the caller surfaces "check your broker app" rather than retry blindly.
- **Timestamps differ by endpoint:** intraday responses are Unix seconds; historical responses have a 315,532,800 s epoch offset. The client handles both.
- **WebSocket:** `src/broker/dhan/websocket.rs` is a stub. Live ticks flow through `algomln::feed::FeedManager`, not Dhan's WebSocket.
- **Quote header deviation:** `POST /marketfeed/quote` is documented as requiring both `access-token` and `client-id` headers; AlgoMLN sends only `access-token` (works in practice but is not strictly to spec).

## Verified against dhanhq.co/docs/v2

All 6 calls match the official Dhan v2 docs (`/orders/`, `/portfolio-and-positions/`, `/market-quote/`, `/historical-data/`, `/instrument-list/`).

Endpoints in the docs that AlgoMLN does **not** use:

- **Orders family:** `PUT /orders/{id}` (modify), `DELETE /orders/{id}` (cancel), `POST /orders/slicing`, `GET /orders`, `GET /orders/{id}`, `GET /orders/external/{correlation-id}`, `GET /trades`, `GET /trades/{order-id}`
- **Positions family:** `GET /holdings`, `POST /positions/convert`, `DELETE /positions`
- **Market feed family:** `POST /marketfeed/ltp`, `POST /marketfeed/ohlc`, `POST /marketfeed/depth`
- **Other:** `POST /charts/expired`, `POST /optionchain`, `POST /marketfeed/full`, `GET /funds`, `GET /statement`, `POST /edis`, `POST /traderControl`, `POST /postback`, plus WebSocket Live Order Update.
