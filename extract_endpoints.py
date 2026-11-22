import yaml
import json

with open('docs_source/dhan-api-v2.yaml', 'r', encoding='utf-8') as f:
    spec = yaml.safe_load(f)

relevant_tags = {'Orders', 'Super Order', 'Conditional and Multi Order', 'Funds'}

endpoints = []
for path, methods in spec.get('paths', {}).items():
    for method, op in methods.items():
        if not isinstance(op, dict): continue
        tags = set(op.get('tags', []))
        if tags.intersection(relevant_tags) or 'orders' in path or 'trade' in path or 'margin' in path:
            endpoints.append({
                'path': path,
                'method': method.upper(),
                'summary': op.get('summary', ''),
                'operationId': op.get('operationId', ''),
                'tags': op.get('tags', []),
                'description': op.get('description', ''),
                'parameters': op.get('parameters', []),
                'requestBody': op.get('requestBody', {}),
                'responses': op.get('responses', {})
            })

print(f"Total relevant endpoints: {len(endpoints)}")
for ep in endpoints:
    print(f"{ep['method']} {ep['path']} -> {ep['summary']} ({ep['tags']})")

with open('relevant_endpoints.json', 'w', encoding='utf-8') as f:
    json.dump(endpoints, f, indent=2)
