with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section(pos, title, length=3000):
    start = text.rfind("\n## ", 0, pos)
    print(f"==================== {title} ====================")
    print(text[start:start+length])

print_section(58454, "Get All Conditional Orders", 2500)
print_section(60609, "Get Conditional Order by ID", 2500)
print_section(27981, "Delete Conditional Order", 2500)
