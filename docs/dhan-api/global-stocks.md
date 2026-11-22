# Global Stocks API map

Live source: [Global Stocks](https://docs.dhanhq.co/api/v2/global-stocks).

The Global Stocks API covers US-stock orders, estimates, trades, market state, holdings, funds and a live feed. Use the separate Global Stocks instrument master from [reference.md](reference.md) to resolve security IDs.

| Method | Endpoint/function | Purpose |
| --- | --- | --- |
| POST | Place Order | Buy/sell US stock with order type, quantity and price inputs |
| PUT | Modify Order | Update pending order by order ID |
| GET | Get Order by ID | Status/details for order ID |
| DELETE | Cancel Order | Cancel pending order ID |
| GET | Get Order Book | Current trading day’s order book |
| POST | Order Estimator | Pre-submit charges estimate |
| POST | Margin Calculator | Pre-submit required margin |
| GET | Get Past Trades | Current-day executed trades |
| GET | Get Past Trades by Security ID | Trades filtered by security ID |
| GET | Get Market Status | US-market session/trading status |
| WebSocket | Global Stocks Live Feed | Real-time trade and OHLC packets |
| GET | Get Holdings | Current US-stock holdings |
| GET | Get Fund Limit | US-stock fund-limit detail |

The overview identifies the operations but does not present endpoint paths or per-field schemas. Use the live function page before wiring a production request; this file preserves every operation enumerated in the current live overview.
