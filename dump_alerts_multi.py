with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section(pos, title, length=4000):
    start = text.rfind("\n## ", 0, pos)
    print(f"==================== {title} ====================")
    print(text[start:start+length])

print_section(223209, "Place Conditional Order", 3500)
print_section(242496, "Place Multi Order", 3500)
