import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

endpoints = [
    r'`POST` https://api\.dhan\.co/v2/orders\b',
    r'`PUT` https://api\.dhan\.co/v2/orders/\{orderId\}',
    r'`DELETE` https://api\.dhan\.co/v2/orders/\{order-id\}',
    r'`GET` https://api\.dhan\.co/v2/orders\b',
    r'`GET` https://api\.dhan\.co/v2/orders/\{order-id\}',
    r'`GET` https://api\.dhan\.co/v2/orders/external/\{correlation-id\}',
    r'`GET` https://api\.dhan\.co/v2/trades\b',
    r'`GET` https://api\.dhan\.co/v2/trades/\{order-id\}',
    r'`POST` https://api\.dhan\.co/v2/orders/slicing\b',
    r'`POST` https://api\.dhan\.co/v2/super/orders\b',
    r'`GET` https://api\.dhan\.co/v2/super/orders\b',
    r'`PUT` https://api\.dhan\.co/v2/super/orders/\{order-id\}',
    r'`DELETE` https://api\.dhan\.co/v2/super/orders/\{order-id\}/\{order-leg\}',
    r'`POST` https://api\.dhan\.co/v2/alerts/orders\b',
    r'`GET` https://api\.dhan\.co/v2/alerts/orders\b',
    r'`GET` https://api\.dhan\.co/v2/alerts/orders/\{alertId\}',
    r'`PUT` https://api\.dhan\.co/v2/alerts/orders/\{alertId\}',
    r'`DELETE` https://api\.dhan\.co/v2/alerts/orders/\{alertId\}',
    r'`POST` https://api\.dhan\.co/v2/alerts/multi/orders\b',
    r'`POST` https://api\.dhan\.co/v2/margincalculator\b',
    r'`POST` https://api\.dhan\.co/v2/margincalculator/multi\b',
]

for ep in endpoints:
    matches = list(re.finditer(ep, text))
    print(f"{ep}: {[m.start() for m in matches]}")
