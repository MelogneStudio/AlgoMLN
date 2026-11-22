with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section(pos, title):
    start = text.rfind("\n## ", 0, pos)
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 4000
    print(f"==================== {title} ====================")
    print(text[start:end])

print_section(132998, "Get Trade Book")
print_section(129476, "Get Trades for Order")
print_section(279937, "Slice Order")
print_section(137, "Calculate Margin")
print_section(3561, "Calculate Multi-Order Margin")
