# DhanHQ API v2 — live documentation map

This reference was derived from the live site at <https://docs.dhanhq.co/api/v2> on 2026-11-22. It deliberately does not use `llms.txt` or `llms-full.txt`.

## Start here

| Topic | Local reference |
| --- | --- |
| Cross-cutting authentication, eligibility, static IP and limits | [platform.md](platform.md) |
| All trading REST endpoints | [trading.md](trading.md) |
| Market-data REST APIs, websockets, option data and analytics | [market-data.md](market-data.md) |
| Account events: postbacks and live order updates | [events.md](events.md) |
| Instrument masters, enums and error codes | [reference.md](reference.md) |
| Global Stocks | [global-stocks.md](global-stocks.md) |
| Python SDK method map | [python-sdk.md](python-sdk.md) |
| Product-wide feature and requirement matrix | [feature-matrix.md](feature-matrix.md) |

## API surface map

```text
Authentication
├─ access token: web UI or token endpoints
├─ OAuth/API-key consent flow
├─ partner consent flow
├─ static-IP management
└─ profile validation

Trading
├─ orders, slicing and trade book
├─ super orders
├─ conditional and multi orders
├─ holdings / positions / conversion / mass exit
├─ EDIS, funds, statements
└─ kill switch and P&L exits

Market data
├─ REST quotes and historical candles
├─ option chain and rolling expired options
├─ analytics
└─ WebSockets: live feed, 20/200 depth and CAS
```

## Common contract

Base URL: `https://api.dhan.co/v2`. REST calls use JSON bodies for POST/PUT and return JSON. Send the current JWT in `access-token`. `dhanClientId` is required by many trading calls. Obtain `securityId` and `exchangeSegment` from the instrument master before submitting any instrument-specific request.

`GET /profile` is the documented low-risk integration probe: it checks access-token validity and account setup.

For endpoint-specific required fields and constraints, follow the linked files above. The original URL for every group is recorded in that file.
