# Rate Limits

DhanHQ API rate limits and best practices

DhanHQ enforces rate limits to ensure fair usage and platform stability.

| API Category | Rate | Per-request cap | Daily limit |
|--------------|------|-----------------|-------------|
| **Order APIs** | 10 req/sec | — | 7,000 |
| **Data APIs** | 5 req/sec | — | 100,000 |
| **Market Quote** | 1 req/sec | 1,000 instruments | — |
| **Option Chain** | 1 req / 3 sec
10 unique scrips / 8 sec | 1 scrip | — |
| **Non Trading APIs** | 20 req/sec
Unlimited per minute and per hour | — | Unlimited |

## Market Quote Limits

- Maximum **1,000 instruments** per request for LTP/OHLC/Quote endpoints
- **1 request per second** across all market quote endpoints

## Option Chain Limits

Two limits apply to Option Chain, and both are enforced together. A request is
rejected if either the per-request rate OR the unique-scrip cap is exceeded.

- **1 request per 3 seconds** per underlying/expiry combination
- **10 unique scrips per 8 seconds** — at most 10 distinct underlyings may be queried in any 8 seconds

## Handling Rate Limits

When you exceed the rate limit, the API returns:

```json
{
  "status": "failure",
  "errorType": "RATE_LIMIT_ERROR",
  "errorCode": "RL001",
  "errorMessage": "Too many requests. Please retry after 1 second."
}
```

### Best Practices

1. **Batch requests** — Use multi-instrument endpoints (e.g., LTP for multiple instruments in one call)
2. **Cache data** — Cache market data locally and refresh at appropriate intervals
3. **Exponential backoff** — When rate limited, wait and retry with increasing delays
4. **Queue orders** — Don't fire all orders simultaneously; space them out

### Python Example with Rate Limiting

```python
import time
from dhanhq import DhanContext, dhanhq

context = DhanContext("client_id", "access_token")
dhan = dhanhq(context)

# Get LTP for multiple instruments in a single call
instruments = [
    ("NSE_EQ", "1333"),
    ("NSE_EQ", "11536"),
    ("NSE_EQ", "2885"),
]
ltp = dhan.get_ltp(instruments)

# Space out order placements
orders = [...]  # your order list
for order in orders:
    response = dhan.place_order(**order)
    print(response)
    time.sleep(0.1)  # 100ms delay between orders
```
