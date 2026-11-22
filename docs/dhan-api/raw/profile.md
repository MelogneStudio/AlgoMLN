source: docs-export.md

# User Profile

User Profile API can be used to check validity of access token and account setup. It is a simple `GET` request and can be a good first validation call during integration.

```bash
curl --location 'https://api.dhan.co/v2/profile' \
  --header 'access-token: {JWT}'
```
