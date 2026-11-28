# LangMelogne Diagnostic Codes

Stable error and warning codes for the LangMelogne (`.mln`) compiler. A code's meaning never changes and codes are never reused.

## Phase 1: Lexer (`E00xx`)

- **E0001** — Invalid character. An unexpected byte, non-ASCII character outside strings/comments, lone carriage return, or stray `%` was encountered. Use `mod(a, b)` instead of `%`.
- **E0002** — Unterminated string. A string literal reached a raw newline or end of file before its closing quote.
- **E0003** — Malformed number. A numeric literal has invalid syntax (for example `1.`, `1__0`, `1_`, `0x10`, or an out-of-range decimal).
- **E0004** — Unknown number suffix. A number was immediately followed by an unrecognised letter suffix (such as `5min` or `3x`).
- **E0005** — Invalid timeframe literal. A timeframe suffix (`m`, `h`, `d`, `w`) was attached to zero or a non-integer number (such as `0m` or `1.5h`).
- **E0006** — Unsupported operator symbol. C-style boolean operators `&&`, `||`, and `!` are not used in LangMelogne; use `and`, `or`, and `not`.
- **E0007** — Invalid string escape. Only `\n`, `\t`, `\\`, `\'`, and `\"` are valid escape sequences in strings.
- **E0010** — Tab in indentation. Indentation must use spaces only (4 spaces per level).
- **E0011** — Indent jumps more than one level. Each indented block must add exactly 4 spaces relative to its parent.
- **E0012** — Indent is not a multiple of 4 spaces. Indentation width must be a multiple of 4 spaces matching an enclosing block on dedent.
- **E0013** — Indented first line. The first non-blank, non-comment line of a program cannot be indented.
- **E0014** — Unmatched closing bracket. A `)` or `]` appeared without a matching opening bracket.
- **E0015** — Unclosed bracket at end of file. An opening `(` or `[` was never closed before EOF.
- **E0017** — Source too large. Input exceeds the 1 MiB (1,048,576 bytes) source size limit.
- **E0021** — Unsupported Python keyword. Python keywords such as `for`, `import`, `lambda`, or `in` are not part of LangMelogne.

## Phase 1: Parser (`E01xx`)

- **E0100** — Unexpected token. A token appeared where the grammar expected a different construct.
- **E0101** — Expected expression. An expression was expected after an operator, `=`, `(`, or keyword.
- **E0102** — Expected identifier. An identifier was expected (for example after `def` or `.`).
- **E0103** — Expected closing bracket. A closing `)` or `]` was expected before the next token.
- **E0110** — Positional argument after keyword argument. All positional arguments must precede keyword arguments in a call.
- **E0111** — Duplicate keyword argument. The same keyword argument name was passed more than once in a single call.
- **E0112** — Empty index. Indexing requires a non-empty expression inside `[...]`.
- **E0113** — Slices are not supported. Slice syntax `a[i:j]` is not supported in LangMelogne.
- **E0114** — Expected timeframe after `@`. The `@` operator requires a timeframe literal such as `1d` or `5m`.
- **E0115** — Duplicate parameter. A `def` parameter list contains two parameters with the same name.
- **E0116** — Required parameter after a default parameter. Parameters without defaults must precede parameters with defaults.
- **E0117** — Nested `def`. Function definitions cannot be nested inside another `def` block.
- **E0118** — `def` block missing its final `return`. Multi-line `def` blocks must end with `return <expr>`.
- **E0119** — Only assignments allowed in a `def` block. Multi-line `def` bodies may contain only assignments before the final `return`.
- **E0120** — Nesting too deep. Expression nesting exceeds the maximum depth of 64.
- **E0130** — `;` is not supported. Put each statement on its own line instead of using semicolons.
- **E0140** — Block not available in this version. Reserved block keywords (`score`, `portfolio`, `use`) are rejected until their later phase.
- **E0150** — `return` outside a `def`, or not last. `return` is only valid as the last statement of a `def` block.
- **E0152** — Expression statement has no effect. Bare expressions are not statements; assign the result to a variable.
- **E0153** — Invalid assignment target. Only plain variable names can appear on the left-hand side of `=`.
- **E0154** — Augmented assignment is not supported. Operators like `+=`, `-=`, `*=`, `/=` are not supported; write `x = x + ...`.
- **E0155** — Unexpected indent. An indented block appeared where no block was opened.
- **E0199** — Too many errors. Diagnostic reporting stopped after 50 errors.

## Phase 2: Checker (`E02xx`, `W02xx`, `E03xx`, `E04xx`)

- **E0200** — Unknown identifier. A variable or function name is not defined in the current scope or builtin registry.
- **E0201** — Redefinition of name. A variable or `def` name is defined more than once in the same scope.
- **E0202** — Use before definition. A variable is referenced before it has been assigned (including self-referential `a = a + 1`).
- **E0203** — Shadowing a builtin. A user variable, `def`, or parameter has the same name as a built-in series or operator.
- **E0210** — Recursive `def`. A `def` function calls itself directly or indirectly.
- **E0220** — Missing required output. A program must define both `entry` and `exit`.
- **W0201** — Unused name. A defined variable or `def` helper is never referenced by `entry`, `exit`, or another used definition.
- **E0300** — Type mismatch. Operand or output types are incompatible (for example, `entry` is not `Bool`, `close and 1`, or `1d + 1`).
- **E0301** — Wrong call arity. A function call has too few or too many arguments, or attempts to call a non-function.
- **E0302** — Unknown keyword argument. A call passes a keyword name not present in the callee's parameter list.
- **E0303** — Argument type mismatch. An argument's type or non-constness does not match the parameter requirement.
- **E0310** — Negative index. Indexing `x[k]` with `k < 0` is rejected because it would read future bars.
- **E0311** — Non-constant index. Indexing `x[k]` requires `k` to be a constant integer.
- **E0401** — Finer clock inside coarser clock. A finer-timeframe series cannot be evaluated inside a coarser-timeframe expression.
- **E0402** — `@` applied to constant. Timeframe resampling `@ tf` cannot be applied to a constant expression.
