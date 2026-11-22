import yaml

with open('docs_source/dhan-api-v2.yaml', 'r', encoding='utf-8') as f:
    spec = yaml.safe_load(f)

print("Paths in YAML:")
for path, methods in spec.get('paths', {}).items():
    if isinstance(methods, dict):
        for method, op in methods.items():
            if isinstance(op, dict):
                print(f"{method.upper()} {path} -> {op.get('summary', '')} | tags: {op.get('tags', [])}")
