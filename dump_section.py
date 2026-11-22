import re

with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def show_section(title, length=3000):
    pos = text.find(f"## {title}\n")
    if pos == -1:
        pos = text.find(f"## {title}")
    if pos != -1:
        print(f"==================== {title} ====================")
        print(text[pos:pos+length])
    else:
        print(f"NOT FOUND: {title}")

# Let's inspect Place Order
show_section("Place Order", 4000)
