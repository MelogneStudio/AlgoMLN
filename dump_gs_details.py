with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

import re

points = [
    ("transEstimate", text.find("/globalstocks/transEstimate")),
    ("marketstatus", text.find("/globalstocks/marketstatus")),
    ("modify_order", text.find("`PUT` https://api.dhan.co/v2/globalstocks/orders")),
    ("get_order_id", text.find("`GET` https://api.dhan.co/v2/globalstocks/orders/{order-id}")),
]

for name, p in points:
    if p != -1:
        start = text.rfind("\n## ", 0, p)
        end = text.find("\n---\n\n## ", p)
        if end == -1: end = p + 2500
        print(f"==================== {name} ====================")
        print(text[start:end])
    else:
        print(f"NOT FOUND: {name}")
