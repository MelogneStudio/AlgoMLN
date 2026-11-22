# Statements

Global Stocks Statements: executed US stock trades with brokerage and charges.

The Global Stocks statements API lets you retrieve past trade history.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [`/globalstocks/trades`](/api/v2/global-stocks/get-past-trades) | Retrieve the current trading day's executed US stock trades for the authenticated user. |
| GET | [`/globalstocks/trades/{security-id}`](/api/v2/global-stocks/get-past-trades-by-scrip) | Retrieve executed US stock trades for the authenticated user filtered by a specific security ID. |
