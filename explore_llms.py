import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    content = f.read()

# Find all headers and any URL occurrences
headers = re.findall(r'^(#{1,3}\s+[^\n]+)', content, re.MULTILINE)
print(f"Total headers: {len(headers)}")

urls = set(re.findall(r'https?://[^\s`"\'\)]+', content))
v2_urls = sorted([u for u in urls if 'v2' in u])
print("v2 URLs:")
for u in v2_urls:
    print(" ", u)
