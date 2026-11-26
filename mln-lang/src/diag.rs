use crate::span::Span;

/// Maximum number of diagnostics reported before emitting `E0199`.
pub const MAX_DIAGNOSTICS: usize = 50;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A compiler diagnostic with a stable code, span, and optional help text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    /// Creates an error diagnostic without help text.
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            help: None,
        }
    }

    /// Attaches a help hint to this diagnostic.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Converts a byte offset in `src` into a 1-based `(line, col)` pair counting Unicode scalar values.
///
/// Treats `\n` and `\r\n` as a single line break. Never panics on arbitrary offsets.
pub fn line_col(src: &str, offset: u32) -> (u32, u32) {
    let mut target = (offset as usize).min(src.len());
    while target > 0 && !src.is_char_boundary(target) {
        target -= 1;
    }

    let mut line: u32 = 1;
    let mut col: u32 = 1;
    let bytes = src.as_bytes();
    let mut i = 0;

    while i < target {
        let ch = src[i..].chars().next().expect("valid slice before target");
        let ch_len = ch.len_utf8();
        if ch == '\r' {
            if i + 1 < target && bytes.get(i + 1) == Some(&b'\n') {
                i += 2;
                line += 1;
                col = 1;
                continue;
            }
            line += 1;
            col = 1;
        } else if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += ch_len;
    }

    (line, col)
}

fn line_slice(src: &str, target_line: u32) -> &str {
    let bytes = src.as_bytes();
    let mut line = 1u32;
    let mut line_start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\r' {
            if line == target_line {
                return &src[line_start..i];
            }
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 2;
            } else {
                i += 1;
            }
            line += 1;
            line_start = i;
        } else if bytes[i] == b'\n' {
            if line == target_line {
                return &src[line_start..i];
            }
            i += 1;
            line += 1;
            line_start = i;
        } else {
            i += 1;
        }
    }

    if line == target_line {
        &src[line_start..]
    } else {
        ""
    }
}

/// Renders a diagnostic with `line:col`, source line, and a caret underline.
pub fn render(diag: &Diagnostic, src: &str) -> String {
    let (line, col) = line_col(src, diag.span.start);
    let kind = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };

    let line_text = line_slice(src, line);
    let start = (diag.span.start as usize).min(src.len());
    let end = (diag.span.end as usize).min(src.len()).max(start);
    let (end_line, end_col) = line_col(src, end as u32);
    let caret_len = if end_line == line && end_col > col {
        (end_col - col) as usize
    } else {
        1
    };

    let mut out = format!(
        "{}[{}]: {} ({}:{})\n  |\n{} | {}\n  | {}{}",
        kind,
        diag.code,
        diag.message,
        line,
        col,
        line,
        line_text,
        " ".repeat((col as usize).saturating_sub(1)),
        "^".repeat(caret_len)
    );
    if let Some(help) = &diag.help {
        out.push_str(&format!("\n  = help: {help}"));
    }
    out
}

/// Sorts diagnostics deterministically by `(span.start, code)` and caps at `MAX_DIAGNOSTICS` + `E0199`.
#[allow(dead_code)]
pub(crate) fn finalize_diagnostics(diags: &mut Vec<Diagnostic>, eof_offset: u32) {
    diags.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| a.code.cmp(b.code))
    });
    if diags.len() > MAX_DIAGNOSTICS {
        diags.truncate(MAX_DIAGNOSTICS);
        diags.push(Diagnostic::error(
            "E0199",
            "too many errors; stopping after 50 diagnostics",
            Span::empty(eof_offset),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_chars_and_crlf() {
        let src = "# é\r\nx = 1";
        // '#': 0, ' ': 1, 'é': 2..4, '\r': 4, '\n': 5, 'x': 6
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 4), (1, 4));
        assert_eq!(line_col(src, 6), (2, 1));
        assert_eq!(line_col(src, 10), (2, 5));
    }

    #[test]
    fn render_prints_line_col_and_caret() {
        let src = "x = @\n";
        let d = Diagnostic::error("E0114", "expected timeframe after `@`", Span::new(4, 5));
        let rendered = render(&d, src);
        assert!(rendered.contains("1:5"));
        assert!(rendered.contains("x = @"));
        assert!(rendered.contains("    ^"));
    }

    #[test]
    fn finalize_caps_at_50_plus_e0199() {
        let mut diags: Vec<Diagnostic> = (0..60)
            .map(|i| Diagnostic::error("E0001", "bad", Span::new(60 - i, 61 - i)))
            .collect();
        finalize_diagnostics(&mut diags, 100);
        assert_eq!(diags.len(), 51);
        assert_eq!(diags[0].span.start, 1);
        assert_eq!(diags[50].code, "E0199");
    }
}
