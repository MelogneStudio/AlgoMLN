import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8', errors='ignore') as f:
    text = f.read()

def get_snippet(heading, length=3000):
    pos = text.find(heading)
    if pos != -1:
        print(f"=== {heading} ===")
        print(text[pos:pos+length])
        print("="*60)
    else:
        print(f"NOT FOUND: {heading}")

get_snippet("## Rate Limits", 2000)
get_snippet("## Trading API Error", 2500)
get_snippet("## Data API Error", 2500)
get_snippet("## Setup Static IP", 2000)
get_snippet("## Testing with Sandbox", 2500)
get_snippet("## Postback Payload", 2500)
