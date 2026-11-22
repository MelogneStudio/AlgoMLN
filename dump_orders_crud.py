with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section(pos, title):
    start = text.rfind("\n## ", 0, pos)
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 4000
    print(f"==================== {title} ====================")
    print(text[start:end])

print_section(213339, "Modify Order")
print_section(13482, "Cancel Order")
print_section(103981, "Get Order Book")
print_section(98894, "Get Order by ID")
print_section(93973, "Get Order by Correlation ID")
