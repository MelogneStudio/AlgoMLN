# dhan-token

Standalone crate for extracting and inspecting claims from a Dhan access token (a JWT).

## IMPORTANT: NO SIGNATURE VERIFICATION

> **Note:** This crate **DOES NOT** verify cryptographic signatures against Dhan's public key or secret.
> It only decodes and parses the JSON payload claims from the JWT.
> It is designed exclusively for client-side claim inspection (such as extracting `client_id`, checking expiration, calculating remaining validity, and scheduling token renewals) and must not be used as an authentication verifier.

## Features & Hygiene

- **Zero Network/Async Overhead:** Extremely lightweight, depends only on `serde`, `serde_json`, `thiserror`, `base64`, and `chrono`.
- **Security-First:** Raw token strings and cryptographic signatures are never stored in `TokenInfo` and never leak into `TokenError` messages or `Debug`/`Display` implementations.
- **Deterministic Testing:** All temporal methods accept `now: DateTime<Utc>` as a parameter so callers and unit tests can deterministically simulate any clock time.
- **Safety Margin Support:** First-class support for safety margins (recommended: 1-2 minutes) to prevent in-flight token expiry.
- **IST Timezone Display:** Built-in Indian Standard Time (UTC+05:30) display and formatting helpers.

## API Overview

```rust
use chrono::{Duration, Utc};
use dhan_token::{TokenInfo, TokenStatus};

let raw_token = std::env::var("DHAN_ACCESS_TOKEN")?;

// Parse token claims without signature verification
let info = TokenInfo::parse(&raw_token)?;

println!("Client ID: {}", info.client_id);
println!("Expires at (UTC): {}", info.expires_at());
println!("Expires at (IST): {}", info.expires_at_ist_string());

// Deterministic status evaluation with 2-minute safety margin
let now = Utc::now();
let margin = Duration::minutes(2);

println!("Time remaining: {}s", info.remaining(now).num_seconds());
println!("Is expired (2m margin): {}", info.is_expired(now, margin));

match info.status(now, margin) {
    TokenStatus::Valid { remaining } => {
        println!("Token is valid for another {}s", remaining.num_seconds());
    }
    TokenStatus::ExpiringSoon { remaining } => {
        println!("Token expires soon (<1h): {}s left", remaining.num_seconds());
    }
    TokenStatus::Expired { since } => {
        println!("Token expired {}s ago", since.num_seconds());
    }
}
```

## Error Codes

The `TokenError` enum uses `thiserror` and exposes stable machine-readable codes via `.code()`:

| Variant | Error Code | Description |
|---|---|---|
| `Empty` | `TOKEN_EMPTY` | Token string is empty or contains only whitespace |
| `NotThreeSegments` | `TOKEN_NOT_THREE_SEGMENTS` | Token does not consist of exactly 3 dot-separated segments |
| `BadBase64` | `TOKEN_BAD_BASE64` | Failed to base64url-decode payload segment |
| `BadJson` | `TOKEN_BAD_JSON` | Failed to parse decoded payload bytes into JSON object |
| `MissingClientId` | `TOKEN_MISSING_CLIENT_ID` | `dhanClientId` claim is missing or not a string/number |
| `MissingExp` | `TOKEN_MISSING_EXP` | `exp` claim is missing from payload |
| `ExpNotNumber` | `TOKEN_EXP_NOT_NUMBER` | `exp` claim is not an integer timestamp |

## Example CLI

A minimal example binary is included to inspect `$env:DHAN_ACCESS_TOKEN`:

```bash
cargo run -p dhan-token --example token-info
```
