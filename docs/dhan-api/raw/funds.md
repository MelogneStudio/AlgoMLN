# Funds

Funds: available balance, fund limits and margin calculator (single order and multiple orders) with the Dhan API.

Users can get details about the fund requirements or available funds (with margin requirements) in their Trading Account.

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | [`/margincalculator`](/api/v2/funds/calculate-margin) | Margin requirement for any order |
| GET | [`/fundlimit`](/api/v2/funds/get-fund-limits) | Retrieve Available Fund Limits |
| POST | [`/margincalculator/multi`](/api/v2/funds/calculate-multi-margin) | Calculate Margin for Multiple Orders |
