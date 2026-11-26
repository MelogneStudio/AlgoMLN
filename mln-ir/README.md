# mln-ir

Stream DAG intermediate representation (`IrGraph`) and common-subexpression elimination (CSE) lowering for **LangMelogne** (`.mln`).

## Public API

- `lower(program: &Program, checked: &CheckedProgram, registry: &OpRegistry) -> Result<IrGraph, IrError>`
- `lower_default(program: &Program) -> Result<IrGraph, IrError>`
- `dump_ir(graph: &IrGraph) -> String`
- `IrGraph`, `IrNode`, `IrOp`, `IrConst`, `IrClock`, `IrTy`, `NodeId`, `SeriesField`

## Example

```rust
use mln_ir::{dump_ir, lower_default};
use mln_lang::parse;

let parsed = parse("a = ema(close, 12)\nb = ema(close, 12)\nentry = a > b\nexit = false");
let graph = lower_default(&parsed.program).unwrap();
let text = dump_ir(&graph);
assert!(text.contains("ema("));
```
