# Python SDK

DhanHQ Python SDK reference and examples

(`dhanhq`)

The official Python SDK for interacting with DhanHQ APIs. Provides a clean, Pythonic interface for all trading and data operations.

## Installation

```bash
pip install dhanhq
```

## Quick Start

```python
from dhanhq import DhanContext, dhanhq

# Initialize with your credentials
context = DhanContext("YOUR_CLIENT_ID", "YOUR_ACCESS_TOKEN")
dhan = dhanhq(context)
```

## Order Management

### Place Order

```python
response = dhan.place_order(
    security_id="1333",
    exchange_segment="NSE_EQ",
    transaction_type="BUY",
    quantity=10,
    order_type="MARKET",
    product_type="INTRADAY"
)
print(response)
```

### Modify Order

```python
response = dhan.modify_order(
    order_id="123456789",
    order_type="LIMIT",
    quantity=15,
    price=1500.00
)
```

### Cancel Order

```python
response = dhan.cancel_order(order_id="123456789")
```

### Get Order Book

```python
orders = dhan.get_order_list()
print(orders)
```

### Get Order by ID

```python
order = dhan.get_order_by_id(order_id="123456789")
```

### Get Order by Correlation ID

```python
order = dhan.get_order_by_corelation_id(correlation_id="my-order-123")
```

### Slice Order (Large Quantities)

```python
response = dhan.place_slice_order(
    security_id="1333",
    exchange_segment="NSE_EQ",
    transaction_type="BUY",
    quantity=5000,
    order_type="MARKET",
    product_type="INTRADAY"
)
```

## Super Order

### Place Super Order

```python
response = dhan.place_super_order(
    security_id="1333",
    exchange_segment="NSE_EQ",
    transaction_type="BUY",
    quantity=10,
    order_type="LIMIT",
    price=1500.00,
    target_price=1550.00,
    stop_loss_price=1470.00,
    trailing_jump=5.00
)
```

### Modify Super Order

```python
response = dhan.modify_super_order(
    order_id="123456789",
    leg_name="TARGET_LEG",
    price=1560.00
)
```

### Cancel Super Order Leg

```python
response = dhan.cancel_super_order(
    order_id="123456789",
    leg_name="STOP_LOSS_LEG"
)
```

## Portfolio

### Get Holdings

```python
holdings = dhan.get_holdings()
```

### Get Positions

```python
positions = dhan.get_positions()
```

### Convert Position

```python
response = dhan.convert_position(
    security_id="1333",
    exchange_segment="NSE_EQ",
    position_type="LONG",
    convert_qty=10,
    from_product_type="INTRADAY",
    to_product_type="CNC"
)
```

### Exit All Positions

```python
response = dhan.exit_all_positions()
```

## Market Data

### Get LTP

```python
instruments = [("NSE_EQ", "1333"), ("NSE_EQ", "11536")]
ltp = dhan.get_ltp(instruments)
```

### Get OHLC

```python
instruments = [("NSE_EQ", "1333")]
ohlc = dhan.get_ohlc(instruments)
```

### Get Full Quote (with Market Depth)

```python
quote = dhan.get_quote(
    security_id="1333",
    exchange_segment="NSE_EQ"
)
```

## Historical Data

### Daily Data

```python
from datetime import datetime, timedelta

data = dhan.get_historical_data(
    security_id="1333",
    exchange_segment="NSE_EQ",
    instrument_type="EQUITY",
    from_date=datetime.now() - timedelta(days=30),
    to_date=datetime.now(),
    interval="D"
)
```

### Intraday Data

```python
data = dhan.get_intraday_data(
    security_id="1333",
    exchange_segment="NSE_EQ",
    instrument_type="EQUITY",
    from_date=datetime.now() - timedelta(days=5),
    to_date=datetime.now(),
    interval="5"  # 1, 5, 15, 25, or 60 minutes
)
```

## Option Chain

### Get Option Chain

```python
chain = dhan.get_option_chain(
    underlying_security_id="26000",
    underlying_type="INDEX",
    expiry_date="2026-02-27"
)
```

### Get Expiry List

```python
expiries = dhan.get_expiry_list(
    underlying_security_id="26000",
    underlying_type="INDEX"
)
```

## Funds

### Get Fund Limits

```python
funds = dhan.get_fund_limits()
print(f"Available: {funds['availabelBalance']}")
```

### Margin Calculator

```python
margin = dhan.get_margin_calculator(
    security_id="1333",
    exchange_segment="NSE_EQ",
    transaction_type="BUY",
    quantity=10,
    product_type="INTRADAY",
    price=1500.00
)
```

### Multi-Order Margin Calculator

```python
scripts = [
    {
        "securityId": "26009",
        "exchangeSegment": "NSE_FNO",
        "transactionType": "BUY",
        "quantity": 50,
        "productType": "INTRADAY",
        "price": 45000.00
    },
    {
        "securityId": "26010",
        "exchangeSegment": "NSE_FNO",
        "transactionType": "SELL",
        "quantity": 50,
        "productType": "INTRADAY",
        "price": 45500.00
    }
]
margin = dhan.get_multi_margin_calculator(scripts=scripts)
```

## Supported Values

### Exchange Segments

| Value | Description |
|-------|-------------|
| `NSE_EQ` | NSE Equity |
| `NSE_FNO` | NSE Futures & Options |
| `BSE_EQ` | BSE Equity |
| `BSE_FNO` | BSE Futures & Options |
| `MCX_COMM` | MCX Commodity |

### Product Types

| Value | Description |
|-------|-------------|
| `CNC` | Cash & Carry (delivery) |
| `INTRADAY` | Intraday |
| `MARGIN` | Margin trading |
| `MTF` | Margin Trading Facility |

### Order Types

| Value | Description |
|-------|-------------|
| `LIMIT` | Limit order |
| `MARKET` | Market order |
| `STOP_LOSS` | Stop loss limit |
| `STOP_LOSS_MARKET` | Stop loss market |
