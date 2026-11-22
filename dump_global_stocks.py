with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

import re

endpoints = [
    r'/globalstocks/orders',
    r'/globalstocks/transEstimate',
    r'/globalstocks/marketstatus',
    r'/globalstocks/trades',
]

for ep in endpoints:
    matches = list(re.finditer(ep, text))
    print(f"Matches for {ep}: {len(matches)}")
    for m in matches[:5]:
        p = m.start()
        start = text.rfind("\n## ", 0, p)
        end = text.find("\n---\n\n## ", p)
        if end == -1: end = p + 3000
        print(f"--- MATCH at {p} ---")
        print(text[start:start+1000])
        print("...\n")
