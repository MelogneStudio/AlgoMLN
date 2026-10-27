# AlgoMLN DSL Specification (frontend v0.1)

This file defines the language accepted by this crate. The supplied requirements list constructs, but no complete legacy grammar or sample program; syntax choices below are explicit frontend decisions. No market data, evaluation, order routing, or engine behavior is implemented.

## Lexical contract

Keywords, price fields, indicator names, and crossover names are ASCII case-insensitive. Strategy names, symbols, and index aliases preserve source spelling and are opaque; no Unicode normalization or confusable-character mapping is performed. Unquoted identifiers begin with a Unicode alphabetic scalar or `_` and continue with Unicode alphanumeric scalars or `_`. Quoted identifiers use `"..."`, preserve their contents, and cannot contain quotes, backslashes, or newlines. Use them for symbols such as `"M&M"`, `"BAJAJ-AUTO"`, `"3MINDIA"`, or `"360ONE"`; strategy names can also be quoted. A quoted identifier is accepted only where a name/symbol is expected. Columns count Unicode scalars (including a tab as one scalar, not visual tab stops), not grapheme clusters; spans also retain UTF-8 byte offsets. Space, tab, and Unicode whitespace separate tokens. LF, CRLF, U+2028, U+2029, and NEL are line breaks, including mixed styles; lone CR is `LEX_NEWLINE`. Blank lines are ignored. `#` is rejected as `LEX_CHAR`; comments are not supported. A final newline is optional.

Numbers are unsigned decimal tokens; unary `+`/`-` is accepted by numeric grammar positions only. Integers are digits; decimals require digits on both sides of `.`. No exponent, NaN, infinity, or separator is supported. Leading zeroes are retained. A number adjacent to identifier characters, another dot, or exponent-like text is a `LEX_NUMBER` error. `%` is a separate immediately adjacent suffix; whitespace before it is a parse error. Times require exactly two digits on each side of `:` and are validated as 00:00–23:59. Operators are `< > <= >= == !=`; standalone `=` and `!` are `LEX_OPERATOR`. `#` is `LEX_CHAR`. Punctuation includes `(`, `)`, `,`, `%`, `+`, `-`.

Resource limits: source 1 MiB, 100,000 tokens, and each identifier or numeric/time token 256 UTF-8 bytes. Exceeding a limit returns `LEX_LIMIT`. Parser nesting is capped at 128 constructs (`PARSE_DEPTH`).

## Grammar (EBNF)

```ebnf
file = [strategy_name], trade_in, { declaration | rule }, EOF ;
strategy_name = "STRATEGY", name, newline ;
trade_in = "TRADE_IN", ( symbol, { ",", symbol } | "INDEX", name ), newline ;
declaration = "STOP_LOSS", signed_number, "%", newline
            | "TAKE_PROFIT", signed_number, "%", newline
            | "RISK", ("MAX_ORDERS" | "MAX_POSITIONS"), integer, newline
            | "RISK", "MAX_DAILY_LOSS", signed_number, "%", newline ;
rule = "WHEN", condition, newline, { newline }, action, (newline | EOF) ;
condition = disjunction ;
disjunction = conjunction, { "OR", conjunction } ;
conjunction = unary, { "AND", unary } ;
unary = {"NOT"}, ("(", condition, ")" | predicate) ;
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
time = digit, digit, ":", digit, digit ; name = identifier | quoted_identifier ; symbol = identifier | quoted_identifier ;
quoted_identifier = '"', non_quote_non_backslash_non_newline, '"' ; identifier = letter_or_underscore, { letter_digit_or_underscore } ;
```

`AND` binds more tightly than `OR`; both are left-associative. Repeated `NOT` is permitted and nests right-to-left. `NOT` binds more tightly than either and applies to a parenthesized condition or predicate. Numeric comparisons bind within predicates; chained comparisons are rejected. Function calls and parentheses bind highest. Thus `A AND B OR C` means `(A AND B) OR C`.

## Declarations and AST mapping

`TRADE_IN` is required exactly once and comes first (after optional `STRATEGY name`); the strategy name is one identifier and its header is only legal at the beginning. An index is written `TRADE_IN INDEX alias`; otherwise a comma-separated explicit symbol list is used. Duplicate symbols are retained in order. Global stop/take/risk declarations may appear anywhere after the header, including between rules, but each setting may occur at most once. Rules and their `rule_1`, `rule_2`, … IDs preserve source order. Each rule is `WHEN condition` followed by an action on the next nonempty line. Conditions cannot span lines in v0.1. Each AST decimal retains its original spelling and exact coefficient/scale; numeric equality and bounds use exact base-10 comparison. Serde encodes the AST structure and spelling, not original whitespace/capitalization/parentheses; canonical formatting and exact source reproduction are separate and not provided.

Price fields are enumerated above; `prev_*` denotes a prior observation without defining offset/history availability. All indicators require a positive `u32` integer period. Period zero, negative/fractional periods, and values above `u32::MAX` are rejected; all positive periods are syntactically supported and practical history requirements belong to runtime. Bollinger functions accept `period` and optional positive decimal multiplier; omitted multiplier is represented by effective `2.0`, so omitted and explicit `2.0` have equal ASTs. Other indicators accept exactly one period. Runtime calculation is not part of the frontend. Crossover accepts exactly two indicator invocations; mixed literal/indicator and literal/literal are rejected. `in_position()` and `between(HH:MM, HH:MM)` are represented but require future engine capabilities. Equal endpoints are allowed; start-after-end is rejected because wrap-around is not supported. Timezone/session interpretation remains engine work.

Actions distinguish integer quantity, decimal `WORTH`, BUY capital percentage, SELL position percentage, and `SELL ALL`. The AST uses distinct `BuyCapitalPercentage` and `SellPositionPercentage` variants. Quantities are positive `u64` integer spellings; worth is a positive exact decimal amount in an unspecified account-currency unit; percentages are exact decimal `(0,100]`. Stop/take/risk daily-loss are `(0,100]`. Risk integer limits are positive `u64`. A strategy needs one or more rules after validation. Explicit source ASTs may be built before validation and are not guaranteed valid; a successful `compile()` guarantees these syntax and frontend semantic invariants. Symbol and index spelling is preserved by the frontend; the future resolver owns any mapping to engine canonical names (including uppercase normalization if required). External symbol/index resolution is out of scope. Engine edge triggering, risk enforcement, crossover state, fills, and stop/take execution remain future work.

## Diagnostics and limits

The lexer/parser stop at the first unrecoverable error and return a structured stage/code/location/span; no partial AST is successful. Error spans cover offending tokens; missing-token errors use a zero-width span at the insertion point (EOF included). The parser requires EOF. Duplicate global settings are parse errors at the second declaration. Validation errors carry stable codes and spans. Unsupported names, punctuation, malformed numbers, trailing tokens and invalid UTF-8 character positions cannot be silently ignored. Fuzzing and property-based tests remain separate follow-up work.

## Known limitations / unresolved source ambiguities

The source brief does not establish exact header spelling, list delimiters, index aliases, comment syntax, action-line boundaries, Bollinger defaults, or lowercase crossover call aliases. v0.1 chooses the forms above. Symbols are opaque identifiers; no exchange normalization is performed. AST serialization preserves numeric spelling, but does not reproduce original source formatting.
