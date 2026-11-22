with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section(pos, title):
    start = text.rfind("\n## ", 0, pos)
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 4000
    print(f"==================== {title} ====================")
    print(text[start:end])

print_section(137, "Calculate Margin")
print_section(279937, "Slice Order")
