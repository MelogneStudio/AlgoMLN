import os
import re

with open('docs-export.md', 'r', encoding='utf-8') as f:
    export_text = f.read()

# Load raw files
raw_files = {}
for r, dirs, files in os.walk('.'):
    for f in files:
        if f.endswith('.md'):
            rel = os.path.relpath(os.path.join(r, f), '.').replace('\\', '/')
            if rel in ['docs-export.md', 'dhan-api-docs.md', 'MANIFEST.md']:
                continue
            with open(rel, 'r', encoding='utf-8', errors='ignore') as fp:
                c = fp.read()
            raw_files[rel] = c

items_to_check = [
    "Sandbox",
    "Feature Matrix",
    "Annexure",
    "Order Update",
    "Postback",
    "Rate Limits",
    "Manage Kill Switch",
    "Kill Switch Status",
    "Configure P&L Exit",
    "Get P&L Exit",
    "Stop P&L Exit",
    "Modify Order",
    "Slice Order",
    "Get Trades",
    "Get Trades for Order",
    "Set IP",
    "Modify IP",
    "Get IP",
    "Access Token",
    "Renew Token",
    "Profile",
]

print("=== Check item occurrences in raw files and docs-export.md ===")
for item in items_to_check:
    # check in raw_files
    matching_raw = []
    for rel, content in raw_files.items():
        # check filename or title or content
        fn = os.path.basename(rel).lower()
        item_slug = item.lower().replace(' ', '-').replace('&', 'and')
        if item_slug in fn or item.lower() in content[:200].lower():
            matching_raw.append(rel)
    
    # check in docs-export
    in_export = (f"## {item}" in export_text) or (f"# {item}" in export_text) or (item.lower() in export_text.lower())
    exact_export_heading = f"## {item}" in export_text or f"# {item}" in export_text
    
    print(f"Item: {item:<25} | In raw: {matching_raw if matching_raw else 'NO MATCH'} | Export heading: {exact_export_heading}")
