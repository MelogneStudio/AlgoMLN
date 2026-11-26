# mln-lang

Front-end crate for **LangMelogne** (`.mln`), containing the lexer, parser, AST, canonical s-expression dump, and diagnostics.

## Public API

- `lex(src: &str) -> LexOutput` — tokenizes UTF-8 source into `tokens` and `diags`.
- `parse(src: &str) -> ParseOutput` — parses `src` into a `Program` (with `Error` recovery nodes) and sorted `diags` (capped at 50 + `E0199`).
- `dump(p: &Program) -> String` — formats the AST in canonical s-expression form.
- `line_col(src: &str, offset: u32) -> (u32, u32)` — converts a byte offset into 1-based `(line, col)` counting Unicode scalar values.
- `render(diag: &Diagnostic, src: &str) -> String` — renders a diagnostic with line/column numbers and a caret underline.
- `ERROR_CODES: &[ErrorCodeInfo]` — registry of stable diagnostic codes.

## Examples

### 1. Crossover strategy

```rust
use mln_lang::{dump, parse};

let src = "\
fast = ema(close, 12)
slow = ema(close, 26)
entry = crossover(fast, slow)
exit = crossunder(fast, slow)
";
let out = parse(src);
assert!(out.diags.is_empty());
assert!(dump(&out.program).contains("(assign entry (call crossover fast slow))"));
```

### 2. Multi-timeframe `def` helper

```rust
use mln_lang::{dump, parse};

let src = "\
def trend_ok(series, n=20):
    avg = ema(series @ 1d, n)
    return series > avg

entry = trend_ok(close, n=50)
exit = close < sma(close, 20)
";
let out = parse(src);
assert!(out.diags.is_empty());
assert!(dump(&out.program).starts_with("(def trend_ok"));
```

### 3. Inspecting diagnostics

```rust
use mln_lang::{line_col, parse, render};

let src = "x = a && b\n";
let out = parse(src);
assert_eq!(out.diags[0].code, "E0006");
assert_eq!(line_col(src, out.diags[0].span.start), (1, 7));
let text = render(&out.diags[0], src);
assert!(text.contains("help:"));
```
