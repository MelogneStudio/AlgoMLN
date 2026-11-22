import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

points = [
    ("POST_orders", 267340),
    ("PUT_orders", 213339),
    ("DELETE_orders", 13482),
    ("GET_orders", 103981),
    ("GET_orders_id", 98894),
    ("GET_orders_correlation", 93973),
    ("GET_trades", 132998),
    ("GET_trades_order", 129476),
    ("POST_slicing", 279937),
    ("POST_super", 271816),
    ("GET_super", 120187),
    ("PUT_super", 216939),
    ("DELETE_super", 15572),
    ("POST_alerts", 223209),
    ("GET_alerts", 58454),
    ("GET_alerts_id", 60609),
    ("PUT_alerts", 193992),
    ("DELETE_alerts", 27981),
    ("POST_multi", 242496),
    ("POST_margin", 137),
    ("POST_margin_multi", 3561),
]

for name, pos in points:
    # find section start before pos
    start = text.rfind("\n## ", 0, pos)
    if start == -1: start = 0
    # find next section start after pos
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 4000
    section_content = text[start:end]
    print(f"=== {name} (length: {len(section_content)}) ===")
    lines = section_content.strip().split("\n")
    print("\n".join(lines[:12]))
    print("...\n")
