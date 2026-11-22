import yaml

with open('docs_source/dhan-api-v2.yaml', 'r', encoding='utf-8') as f:
    text = f.read()

import re
matches = re.findall(r'.{0,100}GoodTillDaysDate.{0,100}', text, re.IGNORECASE)
print(f"YAML matches for GoodTillDaysDate: {len(matches)}")
for m in matches:
    print(m.strip())

matches2 = re.findall(r'.{0,100}forever.{0,100}', text, re.IGNORECASE)
print(f"YAML matches for forever: {len(matches2)}")
for m in matches2:
    print(m.strip())
