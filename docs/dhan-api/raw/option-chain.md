# Option Chain

Option Chain: live option chain with OI, Greeks and IV, plus active expiry dates.

This API gives entire Option Chain of any Option Instrument, across exchanges and segments - for NSE, BSE and MCX traded options. With Option Chain, you get OI, greeks, volume, top bid/ask and price data of all strikes of a particular underlying.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/optionchain`](/api/v2/option-chain/get-option-chain) | Get Option Chain of any instrument |
| POST | [`/optionchain/expirylist`](/api/v2/option-chain/get-expiry-list) | Expiry List for Options of Underlying |

> **Info:** Rate limit for Option Chain API is set to one unique request every 3 seconds. This means you can fetch entire option chain for multiple different underlying instrument or multiple expiries of same instrument concurrently every 3 seconds. A further cap applies across your whole client: at most **10 unique scrips** per **8 seconds**. Both limits are enforced together — a request is rejected if either the per-request rate or the unique-scrip cap is exceeded.
