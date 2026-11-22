with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

pos = text.find('Conditional and Multi Order')
while pos != -1:
    print("Found at:", pos)
    print(text[pos:pos+1500])
    pos = text.find('Conditional and Multi Order', pos+1)
