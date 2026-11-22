# Instrument List

Fetch tradable instrument lists with Security IDs and column descriptions

## Indian Stocks

You can fetch instrument lists for all Indian instruments tradable via Dhan using these URLs:

**Compact:**

```
https://images.dhan.co/api-data/api-scrip-master.csv
```

**Detailed:**

```
https://images.dhan.co/api-data/api-scrip-master-detailed.csv
```

This fetches a CSV with Security IDs and other important details for building with DhanHQ APIs.

---

### Segmentwise List

Fetch a detailed instrument list for a particular exchange and segment:

```bash
curl --location 'https://api.dhan.co/v2/instrument/{exchangeSegment}'
```

> **note:** This fetches instruments for one `exchangeSegment` at a time. The mapping can be found in [Annexure](/api/v2/annexure#exchange-segment).

---

### Column Description

| Detailed Tag | Compact Tag | Description |
|---|---|---|
| `EXCH_ID` | `SEM_EXM_EXCH_ID` | Exchange — `NSE` `BSE` `MCX` |
| `SEGMENT` | `SEM_SEGMENT` | `C` (Currency), `D` (Derivatives), `E` (Equity), `M` (Commodity) |
| `ISIN` | — | ISIN — 12-digit alphanumeric code |
| `INSTRUMENT` | `SEM_INSTRUMENT_NAME` | Instrument defined by Exchange ([Annexure](/api/v2/annexure#instrument)) |
| — | `SEM_EXPIRY_CODE` | Expiry Code for Futures ([Annexure](/api/v2/annexure#expiry-code)) |
| `UNDERLYING_SECURITY_ID` | — | Security ID of underlying (Derivatives) |
| `UNDERLYING_SYMBOL` | — | Symbol of underlying (Derivatives) |
| `SYMBOL_NAME` | `SM_SYMBOL_NAME` | Symbol name |
| — | `SEM_TRADING_SYMBOL` | Exchange trading symbol |
| `DISPLAY_NAME` | `SEM_CUSTOM_SYMBOL` | Dhan display symbol name |
| `INSTRUMENT_TYPE` | `SEM_EXCH_INSTRUMENT_TYPE` | Exchange-defined instrument type |
| `SERIES` | `SEM_SERIES` | Exchange-defined series |
| `LOT_SIZE` | `SEM_LOT_UNITS` | Lot Size |
| `SM_EXPIRY_DATE` | `SEM_EXPIRY_DATE` | Expiry date (Derivatives) |
| `STRIKE_PRICE` | `SEM_STRIKE_PRICE` | Options Strike Price |
| `OPTION_TYPE` | `SEM_OPTION_TYPE` | `CE` (Call) or `PE` (Put) |
| `TICK_SIZE` | `SEM_TICK_SIZE` | Minimum price increment |
| `EXPIRY_FLAG` | `SEM_EXPIRY_FLAG` | `M` (Monthly) or `W` (Weekly) |
| `BRACKET_FLAG` | — | Bracket order — `N` (Not available), `Y` (Allowed) |
| `COVER_FLAG` | — | Cover order — `N` (Not available), `Y` (Allowed) |
| `ASM_GSM_FLAG` | — | ASM/GSM — `N` (No), `R` (Removed), `Y` (Yes) |
| `ASM_GSM_CATEGORY` | — | ASM/GSM category (`NA` if none) |
| `BUY_SELL_INDICATOR` | — | `A` if both Buy/Sell allowed |
| `BUY_CO_MIN_MARGIN_PER` | — | Buy cover order min margin (%) |
| `SELL_CO_MIN_MARGIN_PER` | — | Sell cover order min margin (%) |
| `BUY_CO_SL_RANGE_MAX_PERC` | — | Buy CO max SL range (%) |
| `SELL_CO_SL_RANGE_MAX_PERC` | — | Sell CO max SL range (%) |
| `BUY_CO_SL_RANGE_MIN_PERC` | — | Buy CO min SL range (%) |
| `SELL_CO_SL_RANGE_MIN_PERC` | — | Sell CO min SL range (%) |
| `BUY_BO_MIN_MARGIN_PER` | — | Buy bracket order min margin (%) |
| `SELL_BO_MIN_MARGIN_PER` | — | Sell bracket order min margin (%) |
| `BUY_BO_SL_RANGE_MAX_PERC` | — | Buy BO max SL range (%) |
| `SELL_BO_SL_RANGE_MAX_PERC` | — | Sell BO max SL range (%) |
| `BUY_BO_SL_RANGE_MIN_PERC` | — | Buy BO min SL range (%) |
| `SELL_BO_SL_MIN_RANGE` | — | Sell BO min SL range (%) |
| `BUY_BO_PROFIT_RANGE_MAX_PERC` | — | Buy BO max target range (%) |
| `SELL_BO_PROFIT_RANGE_MAX_PERC` | — | Sell BO max target range (%) |
| `BUY_BO_PROFIT_RANGE_MIN_PERC` | — | Buy BO min target range (%) |
| `SELL_BO_PROFIT_RANGE_MIN_PERC` | — | Sell BO min target range (%) |
| `MTF_LEVERAGE` | — | MTF leverage available (x multiple) for eligible Equity instruments |

---

## Global Stocks

You can fetch the instrument list for global stocks tradable via Dhan using this URL:

```
https://api-global-stocks.dhan.co/api-data/us-stock-scrip-master.csv
```

This fetches a CSV with US stock Security IDs and other important details for building with DhanHQ APIs.

### Column Description

| Column | Description |
|---|---|
| `EXCHANGE` | Internal exchange code, e.g. `INX` for international/US stocks |
| `SEGMENT` | Segment type, `E` = Equity |
| `SCRIP_CODE` | Unique security/instrument ID used by broker/API |
| `ISIN_CODE` | ISIN of the US security |
| `SYMBOL` | Stock/ETF ticker symbol |
| `SYMBOL_NAME` | Official company/fund name |
| `EXCH_SYMBOL` | Symbol on actual US exchange |
| `CUSTOM_SYMBOL` | Display name shown on platform |
| `TRADING_SYMBOL` | Symbol used for trading/orders |
| `FRACTION` | Whether fractional investing is allowed |
| `CUSTOM_EXCH` | Actual exchange: NYSE, NASDAQ, CBOE, NYSE Arca |
| `INSTRUMENT_NAME` | Instrument type, usually `EQUITY` |
| `TICK_SIZE` | Minimum price movement |
| `LOT_SIZE` | Minimum quantity unit |
| `UPPER_LIMIT` | Maximum allowed order price |
| `LOWER_LIMIT` | Minimum allowed order price |
| `UPDATE_DATE` | Last updated timestamp |
