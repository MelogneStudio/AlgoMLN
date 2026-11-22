# Python SDK (`dhanhq`) method map

Live source: [Python SDK](https://docs.dhanhq.co/api/v2/sdks/python).

Install with `pip install dhanhq`; initialize `DhanContext(client_id, access_token)` and `dhanhq(context)`.

| Area | SDK method | Important inputs |
| --- | --- | --- |
| Order | `place_order` | `security_id`, `exchange_segment`, `transaction_type`, `quantity`, `order_type`, `product_type` |
| Order | `modify_order` | `order_id`, `order_type`, `quantity`, `price` |
| Order | `cancel_order` | `order_id` |
| Order | `get_order_list` / `get_order_by_id` / `get_order_by_corelation_id` | no input / `order_id` / `correlation_id` |
| Order | `place_slice_order` | standard order inputs |
| Super | `place_super_order` | order identity plus `price`, `target_price`, `stop_loss_price`, `trailing_jump` |
| Super | `modify_super_order` / `cancel_super_order` | `order_id`, `leg_name`, plus update values |
| Portfolio | `get_holdings`, `get_positions`, `convert_position`, `exit_all_positions` | conversion: `security_id`, `exchange_segment`, `position_type`, `convert_qty`, `from_product_type`, `to_product_type` |
| Quote | `get_ltp`, `get_ohlc` | sequence of `(exchange_segment, security_id)` |
| Quote | `get_quote` | `security_id`, `exchange_segment` |
| Candles | `get_historical_data`, `get_intraday_data` | identity, `instrument_type`, `from_date`, `to_date`, `interval` |
| Options | `get_option_chain`, `get_expiry_list` | `underlying_security_id`, `underlying_type`, and `expiry_date` for chain |
| Funds | `get_fund_limits` | none |
| Margin | `get_margin_calculator`, `get_multi_margin_calculator` | single or a `scripts` array |

SDK accepted-value names follow REST: segments `NSE_EQ`, `NSE_FNO`, `BSE_EQ`, `BSE_FNO`, `MCX_COMM`; products `CNC`, `INTRADAY`, `MARGIN`, `MTF`; types `LIMIT`, `MARKET`, `STOP_LOSS`, `STOP_LOSS_MARKET`.
