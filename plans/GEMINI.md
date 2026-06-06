# TASK: Add PrevOpen, PrevHigh, PrevLow to the AlgoMLN DSL

## READ THIS ENTIRE PROMPT BEFORE TOUCHING ANY FILE.

You are making a small, surgical addition to an existing Rust codebase.
The codebase already works and has 105 passing tests.
Your job is to add three new price field tokens to the DSL — nothing else.

---

## BEFORE YOU WRITE A SINGLE LINE OF CODE, READ THESE FILES IN FULL:

1. `src-tauri/src/strategy/dsl/ast.rs`          — understand PriceField enum
2. `src-tauri/src/strategy/dsl/lexer.rs`         — understand TokenKind enum and keyword matching
3. `src-tauri/src/strategy/dsl/parser.rs`        — understand parse_expr() function
4. `src-tauri/src/strategy/runtime/engine.rs`    — understand eval_expr() function

READ ALL FOUR FILES COMPLETELY BEFORE WRITING ANYTHING.
Do not skim. Do not guess. Read the actual code.

---

## WHAT YOU ARE ADDING

Three new DSL price fields that return the previous candle's values:
- `prev_open`   → candles[candles.len() - 2].open
- `prev_high`   → candles[candles.len() - 2].high
- `prev_low`    → candles[candles.len() - 2].low

`prev_close` already exists and was added recently.
Copy its exact pattern for all three new fields. Do not invent a new pattern.

---

## EXACT FILES TO MODIFY — ONLY THESE 4 FILES, NO OTHERS

### File 1: `src-tauri/src/strategy/dsl/ast.rs`

Find the `PriceField` enum. It currently looks like this:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,   // was added recently
}
```

Add three variants so it becomes:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,
    PrevOpen,
    PrevHigh,
    PrevLow,
}
```

That is the ONLY change to ast.rs.
Do NOT touch IndicatorKind, ConditionNode, ExprNode, ActionNode, StrategyNode,
RuleNode, CompareOp, or any other type in this file.

---

### File 2: `src-tauri/src/strategy/dsl/lexer.rs`

You need to make exactly two additions in this file.

**Addition 1: TokenKind enum**

Find the TokenKind enum. It has a section for price fields that currently
looks something like this (exact names may differ slightly — read the file):

```rust
// Price fields
Close,
Open,
High,
Low,
Volume,
PrevClose,   // was added recently
```

Add three new variants in the same section:

```rust
PrevOpen,
PrevHigh,
PrevLow,
```

**Addition 2: keyword matching**

Find where `prev_close` is matched as a keyword (case-insensitive string match).
It will look something like:

```rust
"prev_close" => TokenKind::PrevClose,
```

Add three new lines immediately after it, following the exact same pattern:

```rust
"prev_open"  => TokenKind::PrevOpen,
"prev_high"  => TokenKind::PrevHigh,
"prev_low"   => TokenKind::PrevLow,
```

Those are the ONLY two changes to lexer.rs.
Do NOT touch any other part of the lexer.
Do NOT change how comments are handled.
Do NOT change how numbers are tokenized.
Do NOT change how operators are tokenized.
Do NOT change any existing keyword.
Do NOT change the Token struct.
Do NOT change LexError.

---

### File 3: `src-tauri/src/strategy/dsl/parser.rs`

Find `parse_expr()`. It has a match arm for `TokenKind::PrevClose` that
looks something like:

```rust
TokenKind::PrevClose => {
    self.advance();
    Ok(ExprNode::PriceField(PriceField::PrevClose))
}
```

Add three new match arms immediately after it, following the exact same pattern:

```rust
TokenKind::PrevOpen => {
    self.advance();
    Ok(ExprNode::PriceField(PriceField::PrevOpen))
}
TokenKind::PrevHigh => {
    self.advance();
    Ok(ExprNode::PriceField(PriceField::PrevHigh))
}
TokenKind::PrevLow => {
    self.advance();
    Ok(ExprNode::PriceField(PriceField::PrevLow))
}
```

That is the ONLY change to parser.rs.
Do NOT touch parse_rule(), parse_condition(), parse_action(),
parse_cross(), parse_not(), or any other function.
Do NOT change rule ID assignment.
Do NOT change how AND/OR conditions are parsed.
Do NOT change error types.

---

### File 4: `src-tauri/src/strategy/runtime/engine.rs`

Find `eval_expr()`. It has a match arm for `PriceField::PrevClose` that
looks something like:

```rust
PriceField::PrevClose => {
    if candles.len() < 2 {
        return Err(EvalError::InsufficientHistory {
            required: 2,
            available: candles.len(),
        });
    }
    Ok(candles[candles.len() - 2].close)
}
```

Add three new match arms immediately after it, following the exact same pattern.
The only difference is the field name (.open, .high, .low):

```rust
PriceField::PrevOpen => {
    if candles.len() < 2 {
        return Err(EvalError::InsufficientHistory {
            required: 2,
            available: candles.len(),
        });
    }
    Ok(candles[candles.len() - 2].open)
}
PriceField::PrevHigh => {
    if candles.len() < 2 {
        return Err(EvalError::InsufficientHistory {
            required: 2,
            available: candles.len(),
        });
    }
    Ok(candles[candles.len() - 2].high)
}
PriceField::PrevLow => {
    if candles.len() < 2 {
        return Err(EvalError::InsufficientHistory {
            required: 2,
            available: candles.len(),
        });
    }
    Ok(candles[candles.len() - 2].low)
}
```

IMPORTANT: The exact shape of the PrevClose arm in your file is the
authoritative pattern. If it looks slightly different from what is shown
above (different error type name, different field access), match whatever
is already there. Do not invent a new pattern.

That is the ONLY change to engine.rs.
Do NOT touch on_candle().
Do NOT touch eval_condition().
Do NOT touch CrossDetector usage.
Do NOT touch TriggerStateMap usage.
Do NOT touch StrategyEngine struct fields.
Do NOT touch StrategyInstance.
Do NOT touch logging calls.

---

## TESTS TO ADD

Add these tests to the EXISTING test blocks in the files below.
Do NOT create new test files. Find the `#[cfg(test)]` block that already
exists at the bottom of each file and add to it.

### In lexer.rs test block, add:

```rust
#[test]
fn tokenizes_prev_open() {
    let src = "WHEN prev_open < 100\nBUY 1";
    let tokens = Lexer::new(src).tokenize().unwrap();
    assert!(tokens.iter().any(|t| t.kind == TokenKind::PrevOpen));
}

#[test]
fn tokenizes_prev_high() {
    let src = "WHEN prev_high > 200\nBUY 1";
    let tokens = Lexer::new(src).tokenize().unwrap();
    assert!(tokens.iter().any(|t| t.kind == TokenKind::PrevHigh));
}

#[test]
fn tokenizes_prev_low() {
    let src = "WHEN prev_low < 150\nBUY 1";
    let tokens = Lexer::new(src).tokenize().unwrap();
    assert!(tokens.iter().any(|t| t.kind == TokenKind::PrevLow));
}
```

### In parser.rs test block, add:

```rust
#[test]
fn parses_prev_open() {
    let node = parse("WHEN prev_open < 100\nBUY 1").unwrap();
    match &node.rules[0].condition {
        ConditionNode::Comparison { left, .. } => {
            assert!(matches!(left, ExprNode::PriceField(PriceField::PrevOpen)));
        }
        _ => panic!("expected comparison"),
    }
}

#[test]
fn parses_prev_high() {
    let node = parse("WHEN close > prev_high\nBUY 1").unwrap();
    match &node.rules[0].condition {
        ConditionNode::Comparison { right, .. } => {
            assert!(matches!(right, ExprNode::PriceField(PriceField::PrevHigh)));
        }
        _ => panic!("expected comparison"),
    }
}

#[test]
fn parses_prev_low() {
    let node = parse("WHEN close < prev_low\nSELL ALL").unwrap();
    match &node.rules[0].condition {
        ConditionNode::Comparison { right, .. } => {
            assert!(matches!(right, ExprNode::PriceField(PriceField::PrevLow)));
        }
        _ => panic!("expected comparison"),
    }
}
```

---

## FILES YOU MUST NOT TOUCH

Do not open, read, or modify any of these files:

- `src-tauri/src/indicators/mod.rs`         ← indicator functions, not your concern
- `src-tauri/src/strategy/execution/paper.rs`   ← paper broker, not your concern
- `src-tauri/src/strategy/execution/order_builder.rs`
- `src-tauri/src/strategy/execution/target.rs`
- `src-tauri/src/strategy/runtime/cross.rs`
- `src-tauri/src/strategy/runtime/trigger_state.rs`
- `src-tauri/src/strategy/runtime/indicator_provider.rs`
- `src-tauri/src/strategy/runtime/context.rs`
- `src-tauri/src/strategy/dsl/validator.rs`
- `src-tauri/src/strategy/logging/log.rs`
- `src-tauri/src/strategy/mod.rs`
- `src-tauri/src/broker/` (anything in here)
- `src-tauri/src/commands/` (anything in here)
- `src-tauri/src/bin/` (anything in here)
- `Cargo.toml`
- `Cargo.lock`

---

## VERIFICATION

After making all changes, run:

```bash
cargo test
```

Required result: ALL existing tests still pass (105 minimum), plus the 6
new tests you added. Zero failures. Zero regressions.

If `cargo test` shows any failure in a test you did not add, you broke
something. Undo that change immediately using git.

Run `git diff --stat` and verify it shows changes to exactly these files:
- `src-tauri/src/strategy/dsl/ast.rs`
- `src-tauri/src/strategy/dsl/lexer.rs`
- `src-tauri/src/strategy/dsl/parser.rs`
- `src-tauri/src/strategy/runtime/engine.rs`

If `git diff --stat` shows ANY other file was modified, undo those changes.
They are not part of this task.

---

## SUMMARY OF ALL CHANGES

Total changes across all files:
- ast.rs: +3 enum variants
- lexer.rs: +3 TokenKind variants, +3 keyword match arms, +3 test functions
- parser.rs: +3 match arms in parse_expr(), +3 test functions
- engine.rs: +3 match arms in eval_expr()

That is everything. Nothing else.