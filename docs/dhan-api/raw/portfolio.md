# Portfolio

Portfolio: your demat holdings, open positions and P&L, plus converting and squaring off positions with the Dhan API.

This API lets you retrieve holdings and positions in your portfolio.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [`/holdings`](/api/v2/portfolio/get-holdings) | Retrieve list of holdings in demat account |
| GET | [`/positions`](/api/v2/portfolio/get-positions) | Retrieve open positions |
| POST | [`/positions/convert`](/api/v2/portfolio/convert-position) | Convert intraday position to delivery or delivery to intraday |
| DELETE | [`/positions`](/api/v2/portfolio/exit-all-positions) | Exit All Positions |
