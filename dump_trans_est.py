with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

pos = text.find('`POST` https://api.dhan.co/v2/globalstocks/transEstimate')
if pos != -1:
    start = text.rfind("\n## ", 0, pos)
    end = text.find("\n---\n\n## ", pos)
    if end == -1: end = pos + 2500
    print(text[start:end])
else:
    print("Not found")
