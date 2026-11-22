with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

import re

for match in re.finditer(r'/alerts/orders', text):
    start = max(0, match.start() - 200)
    end = min(len(text), match.end() + 500)
    print("=== MATCH ===")
    print(text[start:end])
    print("\n")
