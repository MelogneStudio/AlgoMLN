# AlgoMLN — Strategy Engine Architecture v2
## Codex Implementation Brief

> **Guiding principle:** Optimize for correctness and determinism, not speed.
> A strategy that is slightly slower but always produces identical results
> in backtests and live execution is preferable to a faster but more complex engine.

---

## 0. Context and Constraints

This document defines the architecture for the AlgoMLN strategy scripting and execution engine.
It is the implementation specification for Codex.

**Hard constraints:**
- Rust only. No embedded Python, no Rhai, no external interpreters.
- The DSL is a strategy definition language, not a general-purpose language.
- Paper trading is always the default. Live trading is a future phase.
- The visual builder (Phase 5) will compile to the same AST this engine evaluates. Design for this from the start. All AST nodes must be serializable so the visual builder can construct and send them over the Tauri IPC boundary without going through the parser.
- All broker targets (paper, Dhan, future Upstox) use a single execution trait.
- The engine must never panic. All errors must be caught, logged, and evaluation must continue for remaining rules.

**What is already built (do not rebuild):**
- `BrokerClient` trait
- `DhanClient` implementation
- `Candle`, `Tick`, `Quote`, `Order`, `Position`, `Timeframe` data models
- WebSocket manager with tick fan-out
- Indicator functions: `ma`, `ema`, `rsi`, `atr`, `vwap`, `bollinger_bands`
  - Each has signature: `fn indicator(candles: &[Candle], period: usize) -> Vec<f64>`

---

## 1. Crate / Module Structure

```
src/
  strategy/
    mod.rs               # re-exports, StrategyRegistry, lifecycle glue
    dsl/
      mod.rs
      lexer.rs           # Lexer, Token, TokenKind
      parser.rs          # Parser, ParseError
      ast.rs             # all AST node types
      validator.rs       # AstValidator, ValidationError
    runtime/
      mod.rs
      engine.rs          # StrategyEngine — main evaluation loop
      context.rs         # EvalContext — per-candle state
      cross.rs           # CrossDetector
      trigger_state.rs   # TriggerStateMap
      indicator_provider.rs  # IndicatorProvider trait + FullRecomputeProvider
    execution/
      mod.rs
      target.rs          # ExecutionTarget trait
      paper.rs           # PaperBroker
      order_builder.rs   # builds Order from ActionNode
    logging/
      mod.rs
      log.rs             # StrategyLog, LogEntry, LogKind
```

All of this lives under the existing Tauri `src-tauri/src/` tree.
Expose strategy operations to React via Tauri commands in `commands/strategy.rs`.

---

## 2. Core Type Separation: StrategyNode vs StrategyInstance

This is the most important structural decision in the entire architecture.

A `StrategyNode` is a pure description of rules. It contains no symbol, no timeframe, no runtime state.
A `StrategyInstance` binds a strategy to a specific symbol and execution target.

```rust
// ast.rs

/// Pure strategy definition. No symbol, no runtime state.
/// The visual builder constructs this directly.
/// The parser also produces this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyNode {
    pub name: String,
    pub rules: Vec<RuleNode>,
}

/// Runtime binding: one strategy running on one symbol.
/// Created when the user registers a strategy for execution.
pub struct StrategyInstance {
    pub id: String,                          // uuid, stable
    pub strategy: Arc<StrategyNode>,
    pub symbol: String,
    pub timeframe: Timeframe,
    pub status: StrategyStatus,
    pub execution_target: Arc<dyn ExecutionTarget>,
}
```

This separation means one `StrategyNode` can run on NIFTY, BANKNIFTY, and RELIANCE simultaneously — three `StrategyInstance`s, one `StrategyNode`. No duplication.

---

## 3. Strategy Status

Every `StrategyInstance` carries a status. This is required for management from the UI.

```rust
// mod.rs or engine.rs

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StrategyStatus {
    Running,
    Paused,    // receiving candles but not evaluating rules
    Stopped,   // not receiving candles, engine is idle
    Error(String),  // last error message that caused the halt
}
```

The engine checks `status` at the start of `on_candle`. If `Paused` or `Stopped`, it returns an empty vec immediately without evaluating anything.

---

## 4. DSL Specification

### 4.1 Philosophy

The DSL describes trading rules only. It has no variables, no loops, no functions, no imports.
A strategy is a list of rules. A rule is a condition plus an action. That is the entire language.

### 4.2 Full Grammar (EBNF)

```
strategy       = rule+

rule           = "WHEN" condition newline action

condition      = comparison
               | cross_expr
               | position_expr
               | time_window
               | not_expr
               | logical_expr

comparison     = expr compare_op expr

compare_op     = "<" | ">" | "<=" | ">=" | "==" | "!="

logical_expr   = condition "AND" condition
               | condition "OR" condition

not_expr       = "NOT" "(" condition ")"

cross_expr     = "cross_above" "(" expr "," expr ")"
               | "cross_below" "(" expr "," expr ")"

position_expr  = "in_position" "(" ")"

time_window    = "between" "(" time "," time ")"

time           = digit digit ":" digit digit   (* HH:MM, 24h *)

expr           = indicator_call
               | number
               | price_field

indicator_call = indicator_name "(" number ")"

indicator_name = "ema" | "ma" | "rsi" | "atr" | "vwap"
               | "bb_upper" | "bb_lower" | "bb_mid"

price_field    = "close" | "open" | "high" | "low" | "volume"

number         = [0-9]+ ( "." [0-9]+ )?

action         = buy_action | sell_action

buy_action     = "BUY" integer
sell_action    = "SELL" integer | "SELL" "ALL"

integer        = [0-9]+
```

### 4.3 Keyword List

Reserved words (case-insensitive during lexing, normalized to uppercase in token stream):

```
WHEN  BUY  SELL  ALL  AND  OR  NOT  BETWEEN  IN_POSITION
cross_above  cross_below
ema  ma  rsi  atr  vwap  bb_upper  bb_lower  bb_mid
close  open  high  low  volume
```

### 4.4 Implementation Notes

- `NOT` only wraps a parenthesized condition: `NOT (condition)`. It does not negate bare comparisons without parens. This keeps the parser unambiguous.
- `in_position()` and `between(...)` parse as `ConditionNode` variants but are **not evaluated** in v1 — they produce `EvalError::NotYetImplemented` at runtime, which is logged and causes the rule to be skipped. This allows strategies using them to parse and validate correctly while the runtime catches up.
- `SELL ALL` is kept as a distinct token sequence. It must never be collapsed into `SELL <quantity>` by the parser. These are semantically different: `SELL ALL` means "close the entire position at evaluation time", which may be a different quantity every time.

### 4.5 DSL Examples

```
# EMA crossover
WHEN cross_above(ema(20), ema(50))
BUY 10

WHEN cross_below(ema(20), ema(50))
SELL ALL

# RSI oversold/overbought
WHEN rsi(14) < 30
BUY 5

WHEN rsi(14) > 70
SELL ALL

# Bollinger Band breakout
WHEN close > bb_upper(20)
SELL 10

WHEN close < bb_lower(20)
BUY 10

# Compound condition
WHEN ema(9) > ema(21) AND rsi(14) < 60
BUY 5

# Guard against re-entry (in_position parses but is not evaluated in v1)
WHEN cross_above(ema(20), ema(50)) AND NOT (in_position())
BUY 10
```

---

## 5. Lexer

### 5.1 Token Types

```rust
// lexer.rs

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    When,
    Buy,
    Sell,
    All,
    And,
    Or,
    Not,
    Between,
    InPosition,

    // Indicators
    Ema,
    Ma,
    Rsi,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,

    // Price fields
    Close,
    Open,
    High,
    Low,
    Volume,

    // Cross functions
    CrossAbove,
    CrossBelow,

    // Comparison operators
    Lt,     // <
    Gt,     // >
    Lte,    // <=
    Gte,    // >=
    Eq,     // ==
    Neq,    // !=

    // Literals
    Number(f64),
    Integer(usize),
    TimeStr(String),  // "09:20", stored as raw string, parsed in validator

    // Punctuation
    LParen,
    RParen,
    Comma,
    Newline,

    // Control
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}
```

### 5.2 Tokenization Rules

- Strip leading/trailing whitespace per line.
- Blank lines are skipped.
- Lines beginning with `#` (after whitespace stripping) are comments; skip them.
- Keywords are matched case-insensitively. After matching, the token kind is the canonical form — the source casing is not preserved.
- Two-character operators (`<=`, `>=`, `==`, `!=`) are matched before single-character operators. A tokenizer that checks `<` before `<=` will mangle `<=` into `Lt` + `Eq`.
- Time strings: the pattern `\d\d:\d\d` produces `TokenKind::TimeStr`. Validate the hour/minute values in the validator, not the lexer.
- Numbers: if the token contains `.`, produce `TokenKind::Number(f64)`. Otherwise produce `TokenKind::Integer(usize)`. This distinction matters — indicator periods must be integers.
- Unknown characters produce a `LexError` with line + col. Do not skip unknown characters.

### 5.3 Error Type

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}
```

---

## 6. Parser

### 6.1 Approach

Recursive descent. Hand-written. No parser generator.

### 6.2 Parser Struct

```rust
// parser.rs

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self
    pub fn parse(&mut self) -> Result<StrategyNode, ParseError>

    fn parse_rule(&mut self) -> Result<RuleNode, ParseError>
    fn parse_condition(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_primary_condition(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_comparison(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_cross(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_not(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_position_expr(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_time_window(&mut self) -> Result<ConditionNode, ParseError>
    fn parse_expr(&mut self) -> Result<ExprNode, ParseError>
    fn parse_indicator(&mut self, kind: IndicatorKind) -> Result<ExprNode, ParseError>
    fn parse_action(&mut self) -> Result<ActionNode, ParseError>

    fn peek(&self) -> &Token
    fn advance(&mut self) -> &Token
    fn expect(&mut self, kind: TokenKind) -> Result<&Token, ParseError>
    fn is_at_end(&self) -> bool
}
```

### 6.3 Rule ID Assignment

Rule IDs are **not** stored in DSL source text. They are assigned by the parser after all rules are collected, using a deterministic zero-indexed scheme:

```
rule_0, rule_1, rule_2, ...
```

This means the same DSL source always produces the same rule IDs, making log entries stable across re-parses. The visual builder assigns IDs using the same convention when it constructs `RuleNode`s directly.

### 6.4 Error Type

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}
```

---

## 7. AST

All AST nodes must derive `Debug`, `Clone`, `Serialize`, `Deserialize`.
This is the contract between the DSL parser and the visual builder — both produce this structure.

```rust
// ast.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyNode {
    pub name: String,
    pub rules: Vec<RuleNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleNode {
    pub id: String,           // "rule_0", "rule_1", ... assigned by parser
    pub condition: ConditionNode,
    pub action: ActionNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionNode {
    Comparison {
        left: ExprNode,
        op: CompareOp,
        right: ExprNode,
    },
    CrossAbove {
        fast: ExprNode,
        slow: ExprNode,
    },
    CrossBelow {
        fast: ExprNode,
        slow: ExprNode,
    },
    And(Box<ConditionNode>, Box<ConditionNode>),
    Or(Box<ConditionNode>, Box<ConditionNode>),
    Not(Box<ConditionNode>),
    InPosition,               // parses now, evaluated in future version
    TimeWindow {              // parses now, evaluated in future version
        start: NaiveTime,
        end: NaiveTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprNode {
    Indicator(IndicatorCall),
    PriceField(PriceField),
    Literal(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorCall {
    pub kind: IndicatorKind,
    pub period: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndicatorKind {
    Ema,
    Ma,
    Rsi,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareOp {
    Lt, Gt, Lte, Gte, Eq, Neq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionNode {
    Buy { quantity: usize },
    Sell { quantity: usize },
    SellAll,
    // Future: StopLoss { price: f64 }, TakeProfit { price: f64 }
}
```

`NaiveTime` is from the `chrono` crate. It represents a time-of-day without date or timezone.

---

## 8. Validation Layer

Validation runs on the `StrategyNode` after parsing, before registration.
It does not evaluate — it only checks structural correctness.
All errors are collected and returned together. Never abort on the first error.

```rust
// validator.rs

pub struct AstValidator;

impl AstValidator {
    pub fn validate(strategy: &StrategyNode) -> Vec<ValidationError>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub rule_id: String,
    pub message: String,
    pub kind: ValidationErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationErrorKind {
    EmptyStrategy,
    InvalidPeriod { indicator: String, period: usize },
    InvalidQuantity,
    CrossWithLiteral,         // cross_above(30, ema(20)) makes no sense
    DuplicateRuleIds,
    InvalidTimeRange,         // start >= end in TimeWindow
}
```

**Rules to enforce:**

1. Strategy must have at least one rule.
2. All `IndicatorCall.period` values must be > 0.
3. All `Buy { quantity }` and `Sell { quantity }` values must be > 0.
4. In `CrossAbove` and `CrossBelow`, neither `fast` nor `slow` may be `ExprNode::Literal`. A literal value never changes — it cannot cross anything.
5. All rule IDs must be unique within the strategy.
6. In `TimeWindow`, `start` must be strictly before `end`.

A strategy with any validation errors must not be registered for execution.

---

## 9. Indicator Provider

This abstraction exists to allow the indicator computation strategy to be swapped later without changing the engine. In v1, the only implementation is full recomputation from the candle slice. In a future version, an incremental provider can replace it — the engine code does not change.

```rust
// runtime/indicator_provider.rs

pub trait IndicatorProvider: Send + Sync {
    /// Returns the most recent value for the given indicator and period.
    /// Returns None if there is insufficient candle history.
    fn get(&mut self, kind: &IndicatorKind, period: usize, candles: &[Candle]) -> Option<f64>;

    /// Called at the end of each evaluation cycle to allow incremental
    /// providers to update internal state. No-op for FullRecomputeProvider.
    fn advance(&mut self, candle: &Candle) {}
}

/// v1 implementation: recomputes from full candle history every call.
/// The cache prevents recomputation of the same (kind, period) within one cycle.
pub struct FullRecomputeProvider {
    cache: HashMap<(IndicatorKind, usize), f64>,
}

impl FullRecomputeProvider {
    pub fn new() -> Self
    /// Must be called at the start of each evaluation cycle to clear the cache.
    pub fn clear_cache(&mut self)
}
```

The cache in `FullRecomputeProvider` is a within-cycle cache only. It prevents `ema(20)` from being recomputed three times if three rules reference it on the same candle. It is cleared by `clear_cache()` at the start of each `on_candle` call — it does not persist across candles.

---

## 10. Runtime Evaluation Engine

### 10.1 EvalContext

```rust
// runtime/context.rs

pub struct EvalContext<'a> {
    pub candles: &'a [Candle],
    pub current: &'a Candle,    // always candles.last()
}
```

`EvalContext` is a lightweight view. It owns nothing — it borrows the candle slice for one evaluation cycle. The indicator provider is passed separately so it can maintain its cache across rule evaluations within the same cycle.

### 10.2 Expression Evaluation

```rust
fn eval_expr(
    expr: &ExprNode,
    ctx: &EvalContext,
    provider: &mut dyn IndicatorProvider,
) -> Result<f64, EvalError>
```

- `ExprNode::Literal(v)` → return `v`
- `ExprNode::PriceField(f)` → return the corresponding field from `ctx.current`
- `ExprNode::Indicator(call)` → call `provider.get(&call.kind, call.period, ctx.candles)`. If `None`, return `EvalError::InsufficientData { indicator: call.kind.clone(), period: call.period, available: ctx.candles.len() }`.

### 10.3 Condition Evaluation

```rust
fn eval_condition(
    condition: &ConditionNode,
    ctx: &EvalContext,
    provider: &mut dyn IndicatorProvider,
    cross_detector: &CrossDetector,
    rule_id: &str,
) -> Result<bool, EvalError>
```

- `Comparison { left, op, right }` → evaluate both exprs, apply `op`
- `CrossAbove { fast, slow }` → evaluate both exprs, delegate to `CrossDetector::is_cross_above(rule_id, fast_val, slow_val)`
- `CrossBelow { fast, slow }` → evaluate both exprs, delegate to `CrossDetector::is_cross_below(rule_id, fast_val, slow_val)`
- `And(a, b)` → short-circuit: evaluate `a`; if false, return false without evaluating `b`
- `Or(a, b)` → short-circuit: evaluate `a`; if true, return true without evaluating `b`
- `Not(inner)` → evaluate `inner`, return `!result`
- `InPosition` → return `Err(EvalError::NotYetImplemented("in_position"))`
- `TimeWindow { .. }` → return `Err(EvalError::NotYetImplemented("between"))`

### 10.4 EvalError

```rust
#[derive(Debug, Clone, Serialize)]
pub enum EvalError {
    InsufficientData { indicator: IndicatorKind, period: usize, available: usize },
    NotYetImplemented(&'static str),
    OrderBuildFailed(String),
}
```

### 10.5 StrategyEngine

```rust
// runtime/engine.rs

pub struct StrategyEngine {
    instance: StrategyInstance,
    cross_detector: CrossDetector,
    trigger_state: TriggerStateMap,
    indicator_provider: Box<dyn IndicatorProvider>,
    logger: StrategyLogger,
}

impl StrategyEngine {
    pub fn new(instance: StrategyInstance) -> Self

    /// Called on every candle close with the full candle history.
    pub async fn on_candle(&mut self, candles: &[Candle]) -> Vec<LogEntry>

    fn evaluate_rule(
        &mut self,
        rule: &RuleNode,
        ctx: &EvalContext,
    ) -> Result<Option<ActionNode>, EvalError>
}
```

**`on_candle` execution sequence (strictly in this order):**

1. Check `instance.status`. If `Paused` or `Stopped`, return `vec![]`.
2. Call `indicator_provider.clear_cache()`.
3. Build `EvalContext` from the candle slice.
4. For each rule in `instance.strategy.rules`:
   a. Call `eval_condition()`. On `EvalError`, log `LogEntryKind::EvalError`, call `trigger_state.should_fire(rule_id, false)`, continue to next rule.
   b. Call `trigger_state.should_fire(rule_id, condition_result)`.
   c. Log `LogEntryKind::ConditionEvaluated`.
   d. If `should_fire` returned true:
      - Log `LogEntryKind::RuleFired`.
      - Build order via `order_builder::build_order()`. On error, log and continue.
      - Log `LogEntryKind::OrderSubmitted`.
      - Call `instance.execution_target.execute(order).await`.
      - Log `LogEntryKind::OrderExecuted` or `LogEntryKind::OrderFailed`.
5. For each rule that has a `CrossAbove` or `CrossBelow` condition: call `cross_detector.update()` with the current indicator values. This must happen after all rules are evaluated — not inside the rule loop.
6. Call `indicator_provider.advance(current_candle)`.
7. Return `logger.drain_entries()`.

Step 5 is separated from step 4 because the cross detector must see consistent previous values for all rules during a single cycle. If update were called inside the rule loop, a later rule would see updated state from an earlier rule's evaluation.

---

## 11. Cross Detection

### 11.1 State Storage

```rust
// runtime/cross.rs

pub struct CrossDetector {
    prev_values: HashMap<CrossStateKey, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CrossStateKey {
    pub rule_id: String,
    pub side: CrossSide,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CrossSide { Fast, Slow }
```

### 11.2 Logic

```rust
impl CrossDetector {
    /// true only when: fast_prev <= slow_prev AND fast_curr > slow_curr
    pub fn is_cross_above(&self, rule_id: &str, fast_curr: f64, slow_curr: f64) -> bool {
        match (self.get_prev(rule_id, CrossSide::Fast), self.get_prev(rule_id, CrossSide::Slow)) {
            (Some(fp), Some(sp)) => fp <= sp && fast_curr > slow_curr,
            _ => false,  // no previous values: never fire on first candle
        }
    }

    /// true only when: fast_prev >= slow_prev AND fast_curr < slow_curr
    pub fn is_cross_below(&self, rule_id: &str, fast_curr: f64, slow_curr: f64) -> bool {
        match (self.get_prev(rule_id, CrossSide::Fast), self.get_prev(rule_id, CrossSide::Slow)) {
            (Some(fp), Some(sp)) => fp >= sp && fast_curr < slow_curr,
            _ => false,
        }
    }

    /// Must be called once per rule per candle, after all rules are evaluated.
    pub fn update(&mut self, rule_id: &str, fast_curr: f64, slow_curr: f64) {
        self.prev_values.insert(CrossStateKey { rule_id: rule_id.into(), side: CrossSide::Fast }, fast_curr);
        self.prev_values.insert(CrossStateKey { rule_id: rule_id.into(), side: CrossSide::Slow }, slow_curr);
    }

    fn get_prev(&self, rule_id: &str, side: CrossSide) -> Option<f64> {
        self.prev_values.get(&CrossStateKey { rule_id: rule_id.into(), side }).copied()
    }
}
```

**Critical:** `update()` is called in step 5 of `on_candle`, after all rules are evaluated. Never inside the per-rule loop.

---

## 12. Trigger State

```rust
// runtime/trigger_state.rs

pub struct TriggerStateMap {
    states: HashMap<String, bool>,  // rule_id → was condition true last candle
}

impl TriggerStateMap {
    pub fn new() -> Self

    /// Returns true only on false → true transition.
    /// Always updates stored state.
    pub fn should_fire(&mut self, rule_id: &str, is_true_now: bool) -> bool {
        let was_true = *self.states.get(rule_id).unwrap_or(&false);
        self.states.insert(rule_id.to_string(), is_true_now);
        !was_true && is_true_now
    }
}
```

| Previous | Current | Fires? | New state |
|---|---|---|---|
| false | true | yes | true |
| true | true | no | true |
| true | false | no | false |
| false | false | no | false |

`should_fire` must be called for every rule on every candle. On `EvalError`, call `should_fire(rule_id, false)` — treat the error as a non-firing condition.

---

## 13. Execution Target Trait

```rust
// execution/target.rs

#[async_trait]
pub trait ExecutionTarget: Send + Sync {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError>;
    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError>;
    fn is_paper(&self) -> bool;
    fn name(&self) -> &str;
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionError {
    pub message: String,
    pub kind: ExecutionErrorKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum ExecutionErrorKind {
    InsufficientFunds,
    InsufficientPosition,
    BrokerError(String),
}
```

`Order`, `OrderResult`, `Position` are from Phase 1. Import, do not redefine.

---

## 14. Paper Broker

```rust
// execution/paper.rs

pub struct PaperBroker {
    pub symbol: String,
    cash: f64,
    initial_cash: f64,             // for reset()
    positions: HashMap<String, PaperPosition>,
    trade_history: Vec<PaperTrade>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperPosition {
    pub symbol: String,
    pub quantity: i64,
    pub avg_entry_price: f64,
    pub unrealized_pnl: f64,       // updated by update_unrealized(current_price)
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperTrade {
    pub id: String,                // uuid
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: usize,
    pub price: f64,                // fill price = candle close at execution time
    pub rule_id: String,
    pub pnl: Option<f64>,          // Some on sells, None on buys
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperBrokerState {
    pub cash: f64,
    pub initial_cash: f64,
    pub positions: Vec<PaperPosition>,
    pub trade_history: Vec<PaperTrade>,
    pub total_realized_pnl: f64,
}

impl PaperBroker {
    pub fn new(symbol: String, initial_cash: f64) -> Self
    pub fn get_state(&self) -> PaperBrokerState
    pub fn get_position(&self, symbol: &str) -> Option<&PaperPosition>
    pub fn update_unrealized(&mut self, symbol: &str, current_price: f64)
    pub fn reset(&mut self)
}
```

**`execute` for BUY:**
1. `cost = quantity as f64 * price`
2. If `cost > self.cash` → `ExecutionError::InsufficientFunds`
3. `self.cash -= cost`
4. Update `positions[symbol]`: recalculate `avg_entry_price = (prev_qty * prev_avg + quantity * price) / (prev_qty + quantity)`
5. Append to `trade_history` with `pnl: None`

**`execute` for SELL:**
1. If `positions[symbol].quantity < quantity as i64` → `ExecutionError::InsufficientPosition`
2. `realized_pnl = (price - avg_entry_price) * quantity as f64`
3. `self.cash += quantity as f64 * price`
4. Reduce `positions[symbol].quantity`. If zero, remove the entry.
5. Append to `trade_history` with `pnl: Some(realized_pnl)`

**`execute` for SELL ALL:**
- Equivalent to SELL with `quantity = positions[symbol].quantity`. If no position exists, return `ExecutionError::InsufficientPosition`.

Fill price is always `order.price`, which is the candle close at the time the action fires. The `order_builder` sets this.

---

## 15. Order Builder

```rust
// execution/order_builder.rs

pub fn build_order(
    action: &ActionNode,
    symbol: &str,
    current_price: f64,
    current_position: Option<&PaperPosition>,
    rule_id: &str,
) -> Result<Order, OrderBuildError>

#[derive(Debug)]
pub enum OrderBuildError {
    NoPosition,         // SellAll with no open position
    ZeroQuantity,       // should be caught by validator but guard here too
}
```

- `ActionNode::Buy { quantity }` → `Order { side: Buy, quantity, symbol, price: current_price, ... }`
- `ActionNode::Sell { quantity }` → `Order { side: Sell, quantity, symbol, price: current_price, ... }`
- `ActionNode::SellAll` → if `current_position.is_none()` or `quantity == 0`, return `OrderBuildError::NoPosition`. Else build SELL with the position's full quantity.

---

## 16. Logging

All log entries are immutable after construction. No mutation methods on `LogEntry`.

```rust
// logging/log.rs

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub strategy_id: String,
    pub candle_timestamp: DateTime<Utc>,
    pub kind: LogEntryKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum LogEntryKind {
    ConditionEvaluated {
        rule_id: String,
        result: bool,
        prev_state: bool,
        fired: bool,
        indicator_snapshots: Vec<IndicatorSnapshot>,
    },
    RuleFired {
        rule_id: String,
        action: ActionNode,
    },
    OrderSubmitted {
        rule_id: String,
        order: Order,
    },
    OrderExecuted {
        rule_id: String,
        result: OrderResult,
    },
    OrderFailed {
        rule_id: String,
        error: String,
    },
    EvalError {
        rule_id: String,
        error: String,
    },
    StatusChanged {
        from: StrategyStatus,
        to: StrategyStatus,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorSnapshot {
    pub kind: IndicatorKind,
    pub period: usize,
    pub value: f64,
}

pub struct StrategyLogger {
    strategy_id: String,
    entries: Vec<LogEntry>,     // append-only
}

impl StrategyLogger {
    pub fn log(&mut self, kind: LogEntryKind, candle_ts: DateTime<Utc>)
    pub fn get_entries(&self) -> &[LogEntry]
    pub fn drain_entries(&mut self) -> Vec<LogEntry>
}
```

`drain_entries` is called at the end of each `on_candle` cycle. The returned entries are sent to the frontend via a Tauri event.

---

## 17. Strategy Registry

```rust
// strategy/mod.rs

pub struct StrategyRegistry {
    engines: HashMap<String, StrategyEngine>,  // strategy_id → engine
}

impl StrategyRegistry {
    pub fn new() -> Self

    pub fn register(
        &mut self,
        node: StrategyNode,
        symbol: String,
        timeframe: Timeframe,
        target: Arc<dyn ExecutionTarget>,
    ) -> Result<String, RegistrationError>

    pub fn unregister(&mut self, id: &str) -> bool

    pub fn set_status(&mut self, id: &str, status: StrategyStatus) -> bool

    pub fn get_engine_mut(&mut self, id: &str) -> Option<&mut StrategyEngine>

    pub fn list(&self) -> Vec<StrategyMeta>
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyMeta {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub timeframe: Timeframe,
    pub rule_count: usize,
    pub status: StrategyStatus,
    pub is_paper: bool,
}

#[derive(Debug)]
pub enum RegistrationError {
    ValidationFailed(Vec<ValidationError>),
    AlreadyRegistered(String),
}
```

`register` runs `AstValidator::validate` before creating the engine. If validation fails, return `RegistrationError::ValidationFailed` with all errors. The engine is never created for an invalid strategy.

---

## 18. Strategy Lifecycle (Full Flow)

```
User provides DSL text
        │
        ▼
    Lexer::tokenize(source)
    → Vec<Token> or LexError
        │
        ▼
    Parser::parse(tokens)
    → StrategyNode or ParseError
        │  (rule IDs assigned here: rule_0, rule_1, ...)
        ▼
    StrategyRegistry::register(node, symbol, timeframe, target)
        │
        ├── AstValidator::validate(node)
        │   → if errors: return RegistrationError::ValidationFailed (abort)
        │
        ├── Build StrategyInstance { id, strategy: Arc::new(node), symbol, ... }
        ├── Build StrategyEngine { instance, CrossDetector::new(), TriggerStateMap::new(), ... }
        └── Insert into registry, return strategy_id
                │
                ▼
        On each candle close:
        engine.on_candle(&candles)
                │
                ├── Check status (Paused/Stopped → return [])
                ├── indicator_provider.clear_cache()
                ├── Build EvalContext
                │
                ├── For each rule:
                │     eval_condition()
                │     trigger_state.should_fire()
                │     log ConditionEvaluated
                │     if fires:
                │       log RuleFired
                │       build_order()
                │       log OrderSubmitted
                │       execution_target.execute(order).await
                │       log OrderExecuted or OrderFailed
                │
                ├── cross_detector.update() for each cross rule
                ├── indicator_provider.advance(current_candle)
                └── return logger.drain_entries()
```

---

## 19. Tauri Command Interface

```rust
// commands/strategy.rs

/// Parse DSL source into an AST. Does not register or validate.
#[tauri::command]
async fn parse_strategy(source: String) -> Result<StrategyNode, String>

/// Validate an AST. Returns all errors if any.
#[tauri::command]
async fn validate_strategy(node: StrategyNode) -> Result<(), Vec<ValidationError>>

/// Parse, validate, and register a strategy for a symbol.
#[tauri::command]
async fn register_strategy(
    source: String,
    symbol: String,
    timeframe: Timeframe,
    initial_cash: f64,
) -> Result<String, String>   // returns strategy_id

/// Register from a pre-built AST (visual builder path).
#[tauri::command]
async fn register_strategy_from_ast(
    node: StrategyNode,
    symbol: String,
    timeframe: Timeframe,
    initial_cash: f64,
) -> Result<String, String>

#[tauri::command]
async fn unregister_strategy(strategy_id: String) -> bool

#[tauri::command]
async fn set_strategy_status(strategy_id: String, status: StrategyStatus) -> Result<(), String>

#[tauri::command]
async fn list_strategies() -> Vec<StrategyMeta>

#[tauri::command]
async fn get_paper_state(strategy_id: String) -> Result<PaperBrokerState, String>

#[tauri::command]
async fn get_strategy_logs(strategy_id: String) -> Result<Vec<LogEntry>, String>

/// Run a strategy against historical candles. No live state is affected.
#[tauri::command]
async fn run_backtest(
    node: StrategyNode,
    symbol: String,
    candles: Vec<Candle>,
    initial_cash: f64,
) -> Result<BacktestResult, String>

#[derive(Debug, Serialize)]
pub struct BacktestResult {
    pub trade_history: Vec<PaperTrade>,
    pub final_cash: f64,
    pub total_realized_pnl: f64,
    pub total_candles_processed: usize,
    pub logs: Vec<LogEntry>,
}
```

`run_backtest` is the primary integration test path. It does not touch the registry. It creates a temporary `PaperBroker` and `StrategyEngine`, feeds every candle in sequence, and returns the full result. This command should be implemented and tested before any live candle integration.

---

## 20. Dependencies to Add

```toml
# Cargo.toml (src-tauri)
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }  # likely already present
```

---

## 21. Testing Requirements

Every module has unit tests in `#[cfg(test)]` at the bottom of the same file.

**Lexer:**
- tokenizes all examples from section 4.5 without error
- produces `LexError` on unknown character
- skips comments and blank lines
- correctly distinguishes `<=` from `<` followed by `=`
- produces `TokenKind::Integer` for whole numbers, `TokenKind::Number` for decimals

**Parser:**
- parses all examples from section 4.5
- assigns IDs `rule_0`, `rule_1`, etc. in order
- produces `ParseError` on missing action after condition
- produces `ParseError` on malformed condition
- correctly nests `AND` conditions
- parses `NOT (condition)`
- parses `SELL ALL` as `ActionNode::SellAll`, not `Sell { quantity }`

**Validator:**
- rejects empty strategy
- rejects `period = 0` on any indicator
- rejects `quantity = 0` on BUY/SELL
- rejects `CrossAbove` with a `Literal` operand
- rejects `TimeWindow` with `start >= end`
- collects all errors, not just the first

**TriggerStateMap:**
- `false → true` fires
- `true → true` does not fire
- `true → false` does not fire
- `false → false` does not fire
- first call with no prior state: treated as `false → X`

**CrossDetector:**
- no previous values: both `is_cross_above` and `is_cross_below` return false
- exact crossover candle: `is_cross_above` returns true
- candle after crossover (still above): `is_cross_above` returns false
- `is_cross_below` symmetric tests

**PaperBroker:**
- BUY deducts correct cash
- BUY updates `avg_entry_price` correctly on multiple buys
- SELL credits correct cash and correct PnL
- SELL ALL with no position returns `ExecutionError::InsufficientPosition`
- BUY with insufficient funds returns `ExecutionError::InsufficientFunds`
- `reset()` restores to initial state

**StrategyEngine (integration test):**
- build engine with `WHEN rsi(14) < 30 / BUY 5` rule
- feed 20 candles with RSI above 30: zero trades
- feed 1 candle where RSI first drops below 30: exactly 1 trade
- feed 5 candles where RSI stays below 30: zero additional trades
- feed 1 candle where RSI rises above 30, then 1 where it drops below again: exactly 1 more trade

**Backtest command (end-to-end):**
- run `run_backtest` with known candle data and a known strategy
- assert `trade_history` length and PnL values match expected output
- deterministic: same input must produce identical output every time

---

## 22. Implementation Order

Build in this order. Do not proceed to the next phase until the current phase has passing tests.

**Phase A — AST and pipeline**
1. `ast.rs` — all types, derives, no logic
2. `lexer.rs` + tests
3. `parser.rs` + tests
4. `validator.rs` + tests

**Phase B — Execution layer**
5. `target.rs` — trait only
6. `paper.rs` + tests
7. `order_builder.rs`

**Phase C — Runtime**
8. `trigger_state.rs` + tests
9. `cross.rs` + tests
10. `indicator_provider.rs` — trait + `FullRecomputeProvider`
11. `context.rs`
12. `engine.rs` — wires everything together

**Phase D — Backtest**
13. `log.rs`
14. `strategy/mod.rs` — `StrategyRegistry`
15. `run_backtest` command (no UI, test via Rust integration test first)

**Phase E — Tauri commands**
16. All remaining commands in `commands/strategy.rs`
17. Tauri event emission for log drain

---

*End of specification.*
