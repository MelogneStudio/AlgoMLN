with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

import re
matches = re.findall(r'.{0,100}(?:forever|gtt).{0,100}', text, re.IGNORECASE)
print(f"Matches for forever/gtt: {len(matches)}")
for m in matches[:20]:
    print("---")
    print(m.strip())
