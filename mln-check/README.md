# mln-check

Static checker for **LangMelogne** (`.mln`): name resolution, recursion detection, constant folding, type checking, clock validation, `def` call-site instantiation, and warm-up reporting.

## Public API

- `check(program: &Program, registry: &OpRegistry, config: &CheckConfig) -> CheckOutput`
- `check_default(program: &Program) -> CheckOutput`
- `check_with_base(program: &Program, base_clock: Tf) -> CheckOutput`
- `CheckedProgram`, `WarmupReport`, `OutputWarmup`, `DefInstance`, `Ty`, `Clock`, `ConstVal`

## Example

```rust
use mln_check::check_default;
use mln_lang::parse;

let parsed = parse("entry = ema(close, 50) > 1\nexit = close < sma(close, 20)");
assert!(parsed.diags.is_empty());

let checked = check_default(&parsed.program);
assert!(checked.is_ok());
assert_eq!(checked.program.warmup.entry.unwrap().own_clock_bars, 49);
```
