# Market data API map

Live sources: [data overview](https://docs.dhanhq.co/api/v2/data-apis), [market quote](https://docs.dhanhq.co/api/v2/market-quote), [historical](https://docs.dhanhq.co/api/v2/historical-data), [option chain](https://docs.dhanhq.co/api/v2/option-chain), [live feed](https://docs.dhanhq.co/api/v2/live-market-feed).

Data APIs require the appropriate Dhan data subscription. The basic data family allows 5 requests/s; quote endpoints have their own 1 request/s / 1,000 instrument cap.

## REST snapshots and candles

| Method | Endpoint | Inputs / output |
| --- | --- | --- |
| POST | `/marketfeed/ltp` | Instrument collection; returns last-traded price snapshots. Up to 1,000 instruments. |
| POST | `/marketfeed/ohlc` | Instrument collection; returns OHLC snapshots. Up to 1,000 instruments. |
| POST | `/marketfeed/quote` | Instrument collection; returns full quote and depth. Up to 1,000 instruments. |
| POST | `/charts/historical` | Daily candles: `securityId`, `exchangeSegment`, `instrument`, date range. Returns timestamp/O/H/L/C/volume. |
| POST | `/charts/intraday` | Intraday candles: same identity/date inputs plus minute interval. Python docs list `1`, `5`, `15`, `25`, `60` minute intervals. |
| POST | `/charts/rollingoption` | Rolling expired-options history: last five years, strike-wise ATM and up to ten strikes above/below; returns OHLC, IV, volume, OI and spot data. |
| POST | `/optionchain/expirylist` | Underlying security ID and underlying type; returns expiry dates. |
| POST | `/optionchain` | Underlying security ID, underlying type, expiry date; returns strikes, OI, Greeks, volume, top bid/ask and prices. |

The Python examples identify the candle fields as `security_id`, `exchange_segment`, `instrument_type`, `from_date`, `to_date`, `interval`; and option-chain inputs as `underlying_security_id`, `underlying_type`, and (for chain) `expiry_date`.

### Quote request and response schema

All three quote methods require request headers `access-token` and `client-id`. The body is a keyed collection of integer security IDs: `NSE_EQ`, `NSE_FNO`, `BSE_EQ` (each array optional, at least one populated). For example: `{ "NSE_EQ": [14366], "NSE_FNO": [58992], "BSE_EQ": [532822] }`.

- LTP result: `status`, `data.{segment}.{securityId}.last_price`.
- OHLC result: the LTP fields plus `ohlc.open`, `ohlc.high`, `ohlc.low`, `ohlc.close`.
- Full Quote result: `average_price`, aggregate buy/sell quantities, LTP/last quantity/time, circuit limits, net change, volume, OI, OHLC, and five bid/ask depth levels. Each level returns `quantity`, `price`, `orders`.

### Option chain schema

Both option calls require `access-token` and `client-id`. Expiry-list body: `UnderlyingScrip` (required integer) and `UnderlyingSeg` (segment string). It returns `data[]` as `YYYY-MM-DD` dates. Chain body: `UnderlyingScrip`, `UnderlyingSeg`, `Expiry` (`YYYY-MM-DD`; acquire it from the expiry call). It returns underlying `last_price` and an `oc` map keyed by strike. Each `ce`/`pe` has average/LTP, OI, previous close/OI/volume, security ID, best bid/ask price/quantity, volume, implied volatility, and Greeks `delta`, `theta`, `gamma`, `vega`.

## Market Analytics

| Method | Endpoint | Key request inputs |
| --- | --- | --- |
| POST | `/data/technical` | instrument identity, timeframe, requested technical indicator IDs |
| POST | `/data/companyinfo` | one equity instrument plus selected sections: `CO`, `RATIOS`, `SHP` |
| POST | `/data/marketmovers` | instrument group and ranking category |
| POST | live news headline endpoint | category, optional stock filter, `limit` up to 50 (Pro Traders Plan) |

Technical calculations are point-in-time as of the latest closed candle. Timeframes: `1`, `5`, `15`, `D`. Indicator and ranking enum maps are in [reference.md](reference.md).

## Live Market Feed WebSocket

Endpoint: `wss://api-feed.dhan.co?version=2&token=<access-token>&clientId=<client-id>&authType=2`.

- Query fields: `version=2`, JWT `token`, `clientId`, `authType=2`.
- Up to five WebSocket connections per user and 5,000 instruments per connection.
- Send JSON requests; receive little-endian binary frames.
- One subscribe message may carry 100 instruments. Subscription body: `RequestCode`, `InstrumentCount`, `InstrumentList[]`, each item with `ExchangeSegment`, `SecurityId`.
- Feed request codes: ticker subscribe `15`, quote `17`, full packet `21`; complementary unsubscriptions `16`, `18`, `22`; whole-feed disconnect `12`.
- Server pings every 10 seconds. The client must pong; no response for 40 seconds closes the connection.

Normal feed header is 8 bytes: response code (1), message length (2), exchange segment (1), security ID (4). Key payloads: ticker code 2 (LTP + trade time), quote code 4, OI code 5, previous close code 6, full quote/depth code 8. Full packets include five 20-byte depth entries: bid quantity, ask quantity, bid-order count, ask-order count, bid price, ask price.

## 20-level and 200-level market depth

| Service | Endpoint | Capacity | Subscribe body |
| --- | --- | --- | --- |
| 20 depth | `wss://depth-api-feed.dhan.co/twentydepth?token=…&clientId=…&authType=2` | 50 NSE equity/derivative instruments per socket | `RequestCode:23`, `InstrumentCount`, `InstrumentList[]` |
| 200 depth (Pro Traders Plan) | `wss://full-depth-api.dhan.co/twohundreddepth?token=…&clientId=…&authType=2` | one NSE equity/derivative instrument/socket | `RequestCode:23`, `ExchangeSegment`, `SecurityId` |

Both send binary frames. Their header is 12 bytes: message length, response code, exchange segment, security ID, then sequence/row count. Bid/ask packets use code 41/51 and each depth level is 16 bytes: float64 price, uint32 quantity, uint32 order count. 20 depth has 20 levels; 200 depth has 200. Unsubscribe named instruments with code 25; code 41 opens a depth feed and code 11 closes it.

## CAS Live Feed (Pro Traders Plan)

Endpoint: `wss://api-feed.dhan.co?token=<access-token>&clientId=<client-id>&authType=2`. Subscribe with `RequestCode:23`, `InstrumentCount`, `InstrumentList`. It streams closing-auction IEP, buy/sell quantities at IEP, five-level depth, imbalance quantity and reference price, plus a separate indicative index-close packet. Use code 24 to delete a named subscription and code 12 to close the feed.
