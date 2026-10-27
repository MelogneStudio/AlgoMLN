# Lexer fuzzing

Install cargo-fuzz and run `cargo fuzz run lexer` from the repository root. The
harness accepts arbitrary byte input, skips invalid UTF-8 (the public lexer
accepts `&str`), and exercises lexing for all valid Unicode strings.
