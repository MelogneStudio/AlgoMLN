# Reference: instruments, enums and errors

Live sources: [instrument lists](https://docs.dhanhq.co/api/v2/instruments), [annexure](https://docs.dhanhq.co/api/v2/annexure).

## Instrument masters

| Scope | URL |
| --- | --- |
| Indian compact | `https://images.dhan.co/api-data/api-scrip-master.csv` |
| Indian detailed | `https://images.dhan.co/api-data/api-scrip-master-detailed.csv` |
| Per exchange segment | `GET https://api.dhan.co/v2/instrument/{exchangeSegment}` |
| US / Global Stocks | `https://api-global-stocks.dhan.co/api-data/us-stock-scrip-master.csv` |

Use the detailed master as the primary order-validation input. Key fields: exchange, segment, ISIN, Dhan and exchange symbols, security ID, instrument/type/series, lot size, expiry, strike, option type, tick size, weekly/monthly expiry, AMO / product availability flags, price/range limits and MTF leverage. The US master includes the API scrip code, ticker/exchange symbols, fractional availability, actual exchange, tick and lot size, and upper/lower price limits.

## Shared enum values

| Group | Values |
| --- | --- |
| Exchange segment | `IDX_I`, `NSE_EQ`, `NSE_FNO`, `BSE_EQ`, `MCX_COMM`, `BSE_FNO` |
| Product | `CNC`, `INTRADAY`, `MARGIN`, `MTF` |
| Instrument | `INDEX`, `FUTIDX`, `OPTIDX`, `EQUITY`, `FUTSTK`, `OPTSTK`, `FUTCOM`, `OPTFUT` |
| AMO time | `PRE_OPEN`, `OPEN`, `OPEN_30`, `OPEN_60` |
| Expiry code | `1` near, `2` next, `3` far |
| Core order status | `TRANSIT`, `PENDING`, `CLOSED`, `TRIGGERED`, `REJECTED`, `CANCELLED`, `PART_TRADED`, `TRADED` |

Feed request codes: `11` connect, `12` disconnect, `15` ticker subscribe, `16` ticker unsubscribe, `17` quote subscribe, `18` quote unsubscribe, `21` full subscribe, `22` full unsubscribe, `23` full market-depth subscribe, `25` full market-depth unsubscribe. Feed response packet codes: `1` index, `2` ticker, `4` quote, `5` OI, `6` previous close, `7` market status, `8` full, `50` disconnect.

## Conditional Order enum map

Comparison types:

- `TECHNICAL_WITH_VALUE`: needs `indicatorName`, `operator`, `timeFrame`, `comparingValue`.
- `TECHNICAL_WITH_INDICATOR`: needs `indicatorName`, `operator`, `timeFrame`, `comparingIndicatorName`.
- `TECHNICAL_WITH_CLOSE`: needs `indicatorName`, `operator`, `timeFrame`.
- `PRICE_WITH_VALUE`: needs `operator`, `comparingValue`.

Indicators: SMA/EMA periods 5, 10, 20, 50, 100, 200; `BB_UPPER`, `BB_LOWER`, `RSI_14`, `ATR_14`, `STOCHASTIC`, `STOCHRSI_14`, `MACD_26`, `MACD_12`, `MACD_HIST`. Operators: `CROSSING_UP`, `CROSSING_DOWN`, `CROSSING_ANY_SIDE`, `GREATER_THAN`, `LESS_THAN`, `GREATER_THAN_EQUAL`, `LESS_THAN_EQUAL`, `EQUAL`, `NOT_EQUAL`. Statuses: `ACTIVE`, `TRIGGERED`, `EXPIRED`, `CANCELLED`.

## Analytics enum map

Timeframes: `1`, `5`, `15`, `D`. Technical IDs include the MA series above; `RSI_14`, `MACD_HIST`, `STOCH`, `STOCHRSI_14`, `ATR_14`, `ADX_14`, `UO`, `ROC`, `WILLR`; and `PIVOT_CLASSIC`, `PIVOT_FIBONACCI`, `PIVOT_CAMARILLA`. Company-information sections are `CO`, `RATIOS`, `SHP`. News categories include `ALL`, companies, equity/debt markets, IPO, global/economy, currency, commodities, crypto, policy/regulation, infrastructure, trade, investment ideas, real estate, startups, technology and mutual funds.

Market Mover instrument groups cannot be mixed in one request: options `OPTIDX`/`OPTSTK`/`OPTFUT`, futures `FUTIDX`/`FUTSTK`/`FUTCOM`, stocks `EQUITY`. Rankings: options `HIGHEST_OI`, `OI_GAINERS`, `OI_LOSERS`, `TOP_VOLUME`; futures `OI_GAINERS`, `OI_LOSERS`, `TOP_VOLUME`; stocks `PRICE_GAINERS`, `PRICE_LOSERS`.

## Errors

Trading error types: `DH-901` invalid/expired credentials, `DH-902` access/subscription, `DH-903` account, `DH-904` rate, `DH-905` input, `DH-906` order request, `DH-907` data, `DH-908` server, `DH-909` backend network, `DH-910` other. Data errors include 804 instrument cap, 805 too many requests/connections, 806 data subscription missing, 807 token expired, 808 authentication failure, 809 invalid token, 810 client ID invalid, 811 expiry invalid, 812 date invalid, 813 security ID invalid, 814 invalid request.
