with open('docs_source/llms-full.txt', 'r', encoding='utf-8') as f:
    text = f.read()

import re
pos = text.find('GoodTillDaysDate')
if pos != -1:
    print(text[max(0, pos-500):min(len(text), pos+1500)])
else:
    print("Not found")
