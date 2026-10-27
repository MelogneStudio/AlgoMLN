# AlgoMLN DSL Specification (frontend v0.1)

This file defines the language accepted by this crate. The supplied requirements list constructs, but no complete legacy grammar or sample program; syntax choices below are explicit frontend decisions. No market data, evaluation, order routing, or engine behavior is implemented.

## Lexical contract

Keywords are case-insensitive ASCII words; symbols and strategy names preserve spelling. Identifiers begin with a Unicode alphabetic character or `_` and continue with Unicode alphanumeric characters or `_`. Whitespace separates tokens; LF and CRLF are declaration/rule separators. Blank lines are ignored. `#` comments are not supported. Columns are one-based Unicode scalar counts and spans also retain UTF-8 byte offsets. A final newline is optional.

Numbers are unsigned decimal spellings lexically; unary `+`/`-` is accepted in numeric positions. Integers are digits; decimals require digits on both sides of `.`. No exponent, NaN, infinity, leading/trailing decimal point, or separator is supported. Leading zeroes are retained. A number adjacent to identifier characters is an error. `%` is a separate immediately adjacent suffix. Times are `HH:MM`, validated semantically as 00:00–23:59. Operators are `< > <= >= == !=`; standalone `=` and `!` are invalid. Punctuation includes `(`, `)`, `,`, `%`, `+`, `-`.

## Grammar (EBNF)

```ebnf
file = [strategy_name], trade_in, { declaration | rule }, EOF ;
strategy_name = "STRATEGY", identifier, newline ;
trade_in = "TRADE_IN", ( identifier, { ",", identifier } | "INDEX", identifier ), newline ;
declaration = "STOP_LOSS", signed_number, "%", newline
            | "TAKE_PROFIT", signed_number, "%", newline
            | "RISK", ("MAX_ORDERS" | "MAX_POSITIONS"), integer, newline
            | "RISK", "MAX_DAILY_LOSS", signed_number, "%", newline ;
rule = "WHEN", condition, newline, action, newline ;
condition = disjunction ;
disjunction = conjunction, { "OR", conjunction } ;
conjunction = unary, { "AND", unary } ;
unary = ["NOT"], ("(", condition, ")" | predicate) ;
predicate = numeric, comparison, numeric
          | ("CROSS_ABOVE" | "CROSS_BELOW" | cross_function), "(", numeric, ",", numeric, ")"
          | "IN_POSITION", "(", ")"
          | "BETWEEN", "(", time, ",", time, ")" ;
cross_function = "cross_above" | "cross_below" ;
numeric = signed_number | price | indicator ;
price = "close" | "open" | "high" | "low" | "volume"
      | "prev_close" | "prev_open" | "prev_high" | "prev_low" ;
indicator = indicator_name, "(", integer, [",", signed_number], ")" ;
indicator_name = "sma" | "ema" | "rsi" | "rel_vol" | "atr" | "vwap"
               | "bb_upper" | "bb_lower" | "bb_mid" ;
action = "BUY", (integer | signed_number, "%" | signed_number, "WORTH")
       | "SELL", "ALL"
       | "SELL", (integer | signed_number, "%" | signed_number, "WORTH") ;
comparison = "<" | "<=" | ">" | ">=" | "==" | "!=" ;
signed_number = ["+" | "-"], number ; integer = digit, { digit } ;
time = digit, digit, ":", digit, digit ; identifier = letter_or_underscore, { letter_digit_or_underscore } ;
```

`AND` binds more tightly than `OR`; both are left-associative. `NOT` binds more tightly than either and applies to a parenthesized condition or predicate. Numeric comparisons bind within predicates. Function calls and parentheses bind highest. Thus `A AND B OR C` means `(A AND B) OR C`.

## Declarations and AST mapping

`TRADE_IN` is required and comes first (after optional `STRATEGY name`). An index is written `TRADE_IN INDEX alias`; otherwise a comma-separated explicit symbol list is used. Global stop/take/risk declarations may appear anywhere after the header, including between rules, but each setting may occur at most once. Rules and their `rule_1`, `rule_2`, … IDs preserve source order. Each rule is `WHEN condition` followed by an action on the next nonempty line. Conditions cannot span lines in v0.1. Rule/action expressions are represented by typed AST variants. Numeric source spellings are retained as strings for exact decimal round-trip.

Price fields are enumerated above; `prev_*` denotes a prior observation without defining offset/history availability. All indicators require a positive integer period. Bollinger functions accept `period` and optional positive multiplier; default multiplier is 2.0. Other indicators accept exactly one period. Runtime calculation is not part of the frontend. Crossover accepts exactly two indicator invocations; mixed literal/indicator and literal/literal are rejected. `in_position()` and `between(HH:MM, HH:MM)` are represented but require future engine capabilities. Time windows are inclusive syntax with no wrap-around semantics specified.

Actions distinguish integer quantity, decimal `WORTH`, percentage, and `SELL ALL`. Quantities must be positive; worth and percentage must be positive; percentages and stop/take/risk daily-loss are `(0,100]`. Risk integer limits are positive. A strategy needs one or more rules. External symbol/index resolution is out of scope. Engine edge triggering, risk enforcement, crossover state, fills, and stop/take execution remain future work.

## Diagnostics and limits

The lexer/parser stop at the first unrecoverable error and return a structured stage/code/location; no partial AST is successful. The parser requires EOF. Duplicate global settings are parse errors. Validation errors carry stable codes and spans when available. Unsupported names, punctuation, malformed numbers, trailing tokens and invalid UTF-8 character positions cannot be silently ignored. Implementation currently uses recursive descent; a nesting/resource limit and fuzz harness are future hardening work.

## Known limitations / unresolved source ambiguities

The source brief does not establish exact header spelling, list delimiters, index aliases, comment syntax, action-line boundaries, Bollinger defaults, or lowercase crossover call aliases. v0.1 chooses the forms above. Symbols are opaque identifiers; no exchange normalization is performed. Decimal validation currently uses finite `f64` range checks, while AST serialization preserves original spelling.
