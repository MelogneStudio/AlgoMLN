# Market Quote

Market Quote: live LTP, OHLC and full quotes with market depth for up to 1,000 instruments per request.

This API gives you snapshots of multiple instruments at once. You can fetch LTP, Quote or Market Depth of instruments via API which sends real time data at the time of API request.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/marketfeed/ltp`](/api/v2/market-quote/get-ltp) | Get ticker data of instruments |
| POST | [`/marketfeed/ohlc`](/api/v2/market-quote/get-ohlc) | Get OHLC data of instruments |
| POST | [`/marketfeed/quote`](/api/v2/market-quote/get-quote) | Get market depth data of instruments |

> **Info:** You can fetch upto 1000 instruments in single API request with rate limit of 1 request per second.
