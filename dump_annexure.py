with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

pos = text.find('## Annexure')
if pos != -1:
    print(text[pos:pos+4000])
