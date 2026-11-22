with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

def print_section_part(pos, title):
    start = text.rfind("\n## ", 0, pos)
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 5000
    print(f"==================== {title} ====================")
    print(text[start:end])

# Print Super Orders details
print_section_part(271816, "Place Super Order")
print_section_part(120187, "Get Super Orders")
print_section_part(216939, "Modify Super Order")
print_section_part(15572, "Cancel Super Order Leg")
