import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

endpoints = [
    "Place Order", "Modify Order", "Cancel Order", "Get Order Book",
    "Get Order by ID", "Get Order by Correlation ID", "Get Trade Book",
    "Get Trades for Order", "Slice Order", "Place Super Order",
    "Get Super Orders", "Modify Super Order", "Cancel Super Order Leg",
    "Calculate Margin", "Calculate Multi-Order Margin",
    "Place Conditional Order", "Get All Conditional Orders",
    "Get Conditional Order by ID", "Modify Conditional Order",
    "Delete Conditional Order", "Place Multi Order"
]

for ep in endpoints:
    pos = text.find(f"## {ep}\n")
    if pos == -1:
        pos = text.find(f"## {ep}")
    print(f"Endpoint: {ep} -> Found at {pos}")
