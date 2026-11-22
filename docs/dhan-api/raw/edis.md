# EDIS

EDIS: CDSL T-PIN and eDIS authorisation to sell stocks from your demat account with the Dhan API.

To sell holding stocks, one needs to complete the CDSL eDIS flow, generate T-PIN & mark stock to complete the sell action.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | [`/edis/tpin`](/api/v2/edis/generate-tpin) | Generate T-PIN |
| POST | [`/edis/from`](/api/v2/edis/generate-edis-form) | Retrieve escaped html form & enter T-PIN |
| GET | [`/edis/inquire/{isin}`](/api/v2/edis/edis-inquiry) | Inquire the status of stock for edis approval. |
