use std::fmt::{self, Write as _};

use crate::source::{SourceDatabase, SourceSpan};

/// Diagnostic severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
            Severity::Help => "help",
        }
    }
}

/// Structured diagnostic code shared across layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticCode {
    domain: &'static str,
    name: &'static str,
}

impl DiagnosticCode {
    pub const fn new(domain: &'static str, name: &'static str) -> Self {
        Self { domain, name }
    }

    pub const fn domain(self) -> &'static str {
        self.domain
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.domain, self.name)
    }
}

/// Relative importance of a diagnostic label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LabelStyle {
    Primary,
    Secondary,
}

/// Span-level attachment for a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub style: LabelStyle,
    pub span: SourceSpan,
    pub message: String,
}

impl DiagnosticLabel {
    pub fn primary(span: SourceSpan, message: impl Into<String>) -> Self {
        Self {
            style: LabelStyle::Primary,
            span,
            message: message.into(),
        }
    }

    pub fn secondary(span: SourceSpan, message: impl Into<String>) -> Self {
        Self {
            style: LabelStyle::Secondary,
            span,
            message: message.into(),
        }
    }
}

/// Structured diagnostic emitted by compiler and tooling layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<DiagnosticCode>,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    pub fn note(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, message)
    }

    pub fn with_code(mut self, code: DiagnosticCode) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_label(mut self, label: DiagnosticLabel) -> Self {
        self.labels.push(label);
        self
    }

    pub fn with_primary_label(self, span: SourceSpan, message: impl Into<String>) -> Self {
        self.with_label(DiagnosticLabel::primary(span, message))
    }

    pub fn with_secondary_label(self, span: SourceSpan, message: impl Into<String>) -> Self {
        self.with_label(DiagnosticLabel::secondary(span, message))
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }

    /// Render this diagnostic as a plain-text string (no ANSI colors).
    ///
    /// For colored output, use [`crate::render::DiagnosticRenderer`].
    pub fn render(&self, sources: &SourceDatabase) -> String {
        let mut rendered = String::new();
        let _ = write!(rendered, "{}", self.severity.as_str());
        if let Some(code) = self.code {
            let _ = write!(rendered, "[{code}]");
        }
        let _ = writeln!(rendered, ": {}", self.message);

        if let Some(label) = self
            .labels
            .iter()
            .find(|label| label.style == LabelStyle::Primary)
            .or_else(|| self.labels.first())
            && let Some(file) = sources.file(label.span.file())
        {
            let location = file.line_column(label.span.span().start());
            let _ = writeln!(
                rendered,
                " --> {}:{}:{}",
                file.path().display(),
                location.line,
                location.column
            );
            let _ = writeln!(rendered, "  |");
            if let Some(line_text) = file.line_text(location.line - 1) {
                let _ = writeln!(rendered, "{:>2} | {}", location.line, line_text);
                let line_span = file
                    .line_span(location.line - 1)
                    .expect("line already resolved for rendered diagnostic");
                let label_span = label.span.span();
                let max_width = line_span
                    .end()
                    .as_usize()
                    .saturating_sub(label_span.start().as_usize());
                // NOTE: caret width is computed from byte positions (label_span.len() is a
                // byte count), not Unicode character widths.  Wide characters (e.g. CJK) or
                // multi-byte UTF-8 sequences will cause the caret to be wider or narrower
                // than the rendered glyph columns, producing visually misaligned output.
                let caret_width = if label_span.is_empty() {
                    1
                } else {
                    usize::max(1, label_span.len() as usize).min(usize::max(1, max_width))
                };
                let _ = writeln!(
                    rendered,
                    "  | {}{}",
                    " ".repeat(location.column.saturating_sub(1)),
                    "^".repeat(caret_width)
                );
                if !label.message.is_empty() {
                    let _ = writeln!(rendered, "  = {}", label.message);
                }
            }
        }

        // Render all secondary labels.  Each one is shown with its file location, the
        // relevant source line, and a `-`-caret beneath the labelled span, followed by
        // the label message as an indented note line.
        for label in self
            .labels
            .iter()
            .filter(|label| label.style == LabelStyle::Secondary)
        {
            if let Some(file) = sources.file(label.span.file()) {
                let location = file.line_column(label.span.span().start());
                let _ = writeln!(
                    rendered,
                    "  ::: {}:{}:{}",
                    file.path().display(),
                    location.line,
                    location.column,
                );
                let _ = writeln!(rendered, "  |");
                if let Some(line_text) = file.line_text(location.line - 1) {
                    let _ = writeln!(rendered, "{:>2} | {}", location.line, line_text);
                    let label_span = label.span.span();
                    // NOTE: caret width uses byte length — see the same note on the
                    // primary caret above regarding potential misalignment for wide chars.
                    let caret_width = if label_span.is_empty() {
                        1
                    } else {
                        usize::max(1, label_span.len() as usize)
                    };
                    let _ = writeln!(
                        rendered,
                        "  | {}{}",
                        " ".repeat(location.column.saturating_sub(1)),
                        "-".repeat(caret_width)
                    );
                    if !label.message.is_empty() {
                        let _ = writeln!(
                            rendered,
                            "  | {}note: {}",
                            " ".repeat(location.column.saturating_sub(1)),
                            label.message
                        );
                    }
                }
            }
        }

        for note in &self.notes {
            let _ = writeln!(rendered, "note: {note}");
        }

        for h in &self.help {
            let _ = writeln!(rendered, "help: {h}");
        }

        rendered.trim_end().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceDatabase;

    #[test]
    fn renders_primary_and_secondary_labels() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("sample.aivi", "signal counter = 0\n");
        let file = &sources[file_id];

        let rendered = Diagnostic::error("top-level syntax error")
            .with_code(DiagnosticCode::new("syntax", "unexpected-token"))
            .with_primary_label(
                file.source_span(0..3),
                "expected a declaration keyword here",
            )
            .with_secondary_label(
                file.source_span(4..11),
                "this binding belongs to the malformed declaration",
            )
            .with_note("parser stayed in the Milestone 1 surface layer")
            .render(&sources);

        assert!(rendered.contains("error[syntax::unexpected-token]: top-level syntax error"));
        assert!(rendered.contains(" --> sample.aivi:1:1"));
        assert!(rendered.contains("signal counter = 0"));
        assert!(rendered.contains("expected a declaration keyword here"));
        // Secondary labels must be rendered, not silently dropped.
        assert!(
            rendered.contains("this binding belongs to the malformed declaration"),
            "secondary label message missing from rendered output:\n{rendered}"
        );
        assert!(rendered.contains("parser stayed in the Milestone 1 surface layer"));
    }

    #[test]
    fn renders_help_hints() {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("sample.aivi", "value x = 42\n");
        let file = &sources[file_id];

        let rendered = Diagnostic::error("type mismatch")
            .with_primary_label(file.source_span(10..12), "expected Text, found Int")
            .with_help("try wrapping this in `toString`")
            .render(&sources);

        assert!(rendered.contains("help: try wrapping this in `toString`"));
    }

    #[test]
    fn help_field_defaults_empty() {
        let diag = Diagnostic::error("test");
        assert!(diag.help.is_empty());
    }
}
