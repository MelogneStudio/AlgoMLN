import re
import yaml

with open('docs_source/llms-full.txt', 'r', encoding='utf-8', errors='ignore') as f:
    text = f.read()

print('--- WEBSOCKETS ---')
ws_matches = set(re.findall(r'wss?://[^\s\"\'\<\>]+', text))
for m in sorted(ws_matches):
    print(m)

print('\n--- ALL PATHS IN YAML ---')
with open('docs_source/dhan-api-v2.yaml', 'r', encoding='utf-8') as f:
    data = yaml.safe_load(f)

for path, methods in sorted(data.get('paths', {}).items()):
    for method in methods:
        desc = methods[method].get('summary', '') or methods[method].get('description', '')
        print(f"{method.upper():6} {path:35} {desc[:50]}")
