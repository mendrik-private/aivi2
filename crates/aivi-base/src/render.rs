//! ANSI-colored diagnostic renderer with Ghostty-inspired palette.
//!
//! Produces rich, human-readable diagnostic output with:
//! - Colored severity headers and diagnostic codes
//! - Unicode box-drawing gutters (│, ╭, ╰)
//! - Multi-line span rendering with underlines
//! - Primary (`^`) and secondary (`─`) caret styles
//! - Help hints in green
//! - Color auto-detection (isatty + `NO_COLOR` / `FORCE_COLOR`)

use std::fmt::Write as _;
use std::io::{self, IsTerminal};

use crate::diagnostic::{Diagnostic, LabelStyle, Severity};
use crate::source::{SourceDatabase, SourceSpan};

/// Whether to emit ANSI color escapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Detect from terminal capabilities (isatty + env vars).
    Auto,
    /// Always emit ANSI escapes.
    Always,
    /// Never emit ANSI escapes (plain text).
    Never,
}

impl Default for ColorMode {
    fn default() -> Self {
        Self::Auto
    }
}

// ── ANSI escape helpers ─────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const UNDERLINE: &str = "\x1b[4m";

// Ghostty default palette — truecolor (24-bit) for maximum fidelity.
// These are Ghostty's actual default ANSI color mappings.
const RED: &str = "\x1b[38;2;204;36;29m"; // error headers, primary carets
const YELLOW: &str = "\x1b[38;2;215;153;33m"; // warning headers, warning carets
const GREEN: &str = "\x1b[38;2;152;151;26m"; // help text
const CYAN: &str = "\x1b[38;2;104;157;106m"; // note headers
const BLUE: &str = "\x1b[38;2;69;133;136m"; // secondary carets, line numbers
const MAGENTA: &str = "\x1b[38;2;177;98;134m"; // diagnostic codes

// ── Style composites ────────────────────────────────────────────────────────

struct Style {
    prefix: &'static [&'static str],
}

impl Style {
    const fn new(prefix: &'static [&'static str]) -> Self {
        Self { prefix }
    }

    fn paint(&self, color: bool, text: &str) -> String {
        if !color {
            return text.to_owned();
        }
        let mut s = String::with_capacity(32 + text.len());
        for p in self.prefix {
            s.push_str(p);
        }
        s.push_str(text);
        s.push_str(RESET);
        s
    }
}

const STYLE_ERROR: Style = Style::new(&[BOLD, RED]);
const STYLE_WARNING: Style = Style::new(&[BOLD, YELLOW]);
const STYLE_NOTE: Style = Style::new(&[BOLD, CYAN]);
const STYLE_HELP: Style = Style::new(&[BOLD, GREEN]);
const STYLE_CODE: Style = Style::new(&[DIM, MAGENTA]);
const STYLE_GUTTER: Style = Style::new(&[BOLD, BLUE]);
const STYLE_PATH: Style = Style::new(&[UNDERLINE]);
const STYLE_PRIMARY_CARET_ERROR: Style = Style::new(&[BOLD, RED]);
const STYLE_PRIMARY_CARET_WARNING: Style = Style::new(&[BOLD, YELLOW]);
const STYLE_SECONDARY_CARET: Style = Style::new(&[BOLD, BLUE]);
const STYLE_LABEL_MESSAGE: Style = Style::new(&[ITALIC]);
const STYLE_BOLD: Style = Style::new(&[BOLD]);

// ── Color detection ─────────────────────────────────────────────────────────

fn should_colorize(mode: ColorMode, stream_is_tty: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            if std::env::var_os("NO_COLOR").is_some() {
                return false;
            }
            if std::env::var_os("FORCE_COLOR").is_some() {
                return true;
            }
            stream_is_tty
        }
    }
}

fn stderr_is_tty() -> bool {
    io::stderr().is_terminal()
}

// ── Renderer ────────────────────────────────────────────────────────────────

/// Renders [`Diagnostic`] values as richly-formatted ANSI-colored strings.
pub struct DiagnosticRenderer {
    color: bool,
}

impl DiagnosticRenderer {
    /// Create a renderer from the given color mode.
    pub fn new(mode: ColorMode) -> Self {
        Self {
            color: should_colorize(mode, stderr_is_tty()),
        }
    }

    /// Create a renderer that always produces plain text (no ANSI).
    pub fn plain() -> Self {
        Self { color: false }
    }

    /// Create a renderer that always produces colored output.
    pub fn colored() -> Self {
        Self { color: true }
    }

    /// Render a single diagnostic to a string.
    pub fn render(&self, diag: &Diagnostic, sources: &SourceDatabase) -> String {
        let mut out = String::with_capacity(512);
        self.write_diagnostic(&mut out, diag, sources);
        out.trim_end().to_owned()
    }

    /// Render multiple diagnostics separated by blank lines.
    pub fn render_all<'a>(
        &self,
        diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
        sources: &SourceDatabase,
    ) -> String {
        let mut out = String::with_capacity(2048);
        let mut first = true;
        for diag in diagnostics {
            if !first {
                out.push('\n');
            }
            self.write_diagnostic(&mut out, diag, sources);
            first = false;
        }
        out.trim_end().to_owned()
    }

    fn write_diagnostic(&self, out: &mut String, diag: &Diagnostic, sources: &SourceDatabase) {
        self.write_header(out, diag);
        self.write_snippets(out, diag, sources);
        self.write_notes(out, diag);
        self.write_help(out, diag);
        // Trailing newline for separation.
        out.push('\n');
    }

    // ── Header ──────────────────────────────────────────────────────────

    fn write_header(&self, out: &mut String, diag: &Diagnostic) {
        let severity_style = match diag.severity {
            Severity::Error => &STYLE_ERROR,
            Severity::Warning => &STYLE_WARNING,
            Severity::Note => &STYLE_NOTE,
            Severity::Help => &STYLE_HELP,
        };

        out.push_str(&severity_style.paint(self.color, diag.severity.as_str()));

        if let Some(code) = diag.code {
            out.push_str(&STYLE_CODE.paint(self.color, &format!("[{code}]")));
        }

        out.push_str(&STYLE_BOLD.paint(self.color, &format!(": {}", diag.message)));
        out.push('\n');
    }

    // ── Source snippets ─────────────────────────────────────────────────

    fn write_snippets(&self, out: &mut String, diag: &Diagnostic, sources: &SourceDatabase) {
        if diag.labels.is_empty() {
            return;
        }

        // Group labels by file.
        let mut file_groups: Vec<(SourceSpan, Vec<&crate::diagnostic::DiagnosticLabel>)> =
            Vec::new();
        for label in &diag.labels {
            if let Some(group) = file_groups
                .iter_mut()
                .find(|(key, _)| key.file() == label.span.file())
            {
                group.1.push(label);
            } else {
                file_groups.push((label.span, vec![label]));
            }
        }

        let mut is_first_file = true;

        for (_representative, labels) in &file_groups {
            // Pick the primary label for this file group, or the first label.
            let anchor = labels
                .iter()
                .find(|l| l.style == LabelStyle::Primary)
                .or_else(|| labels.first())
                .unwrap();

            let Some(file) = sources.file(anchor.span.file()) else {
                continue;
            };

            let location = file.line_column(anchor.span.span().start());

            // ╭─ path:line:col
            let arrow = if is_first_file {
                STYLE_GUTTER.paint(self.color, "╭─")
            } else {
                STYLE_GUTTER.paint(self.color, "├─")
            };
            let path_str = format!(
                " {}:{}:{}",
                file.path().display(),
                location.line,
                location.column
            );
            let _ = writeln!(out, " {arrow}{}", STYLE_PATH.paint(self.color, &path_str));

            // Collect all lines we need to show.
            let mut line_labels: std::collections::BTreeMap<
                usize,
                Vec<&crate::diagnostic::DiagnosticLabel>,
            > = std::collections::BTreeMap::new();
            for label in labels {
                let loc = file.line_column(label.span.span().start());
                line_labels.entry(loc.line).or_default().push(label);

                // If the span crosses lines, also add the end line.
                let end_loc = file.line_column(label.span.span().end());
                if end_loc.line != loc.line {
                    // For multi-line spans, we show start and end lines.
                    line_labels.entry(end_loc.line).or_default();
                }
            }

            let lines: Vec<usize> = line_labels.keys().copied().collect();

            // Show each relevant line with its labels.
            let max_line_num = lines.last().copied().unwrap_or(1);
            let gutter_width = digit_count(max_line_num);

            // Empty gutter line.
            let _ = writeln!(
                out,
                " {} {}",
                " ".repeat(gutter_width),
                STYLE_GUTTER.paint(self.color, "│")
            );

            let mut prev_line: Option<usize> = None;

            for &line_num in &lines {
                // Show ellipsis if there's a gap.
                if let Some(prev) = prev_line {
                    if line_num > prev + 2 {
                        let _ = writeln!(
                            out,
                            " {}",
                            STYLE_GUTTER.paint(self.color, &format!("{:>gutter_width$} ·", "…"))
                        );
                    } else if line_num > prev + 1 {
                        // Show the intermediate line.
                        let mid = prev + 1;
                        if let Some(text) = file.line_text(mid - 1) {
                            let _ = writeln!(
                                out,
                                " {} {} {}",
                                STYLE_GUTTER.paint(
                                    self.color,
                                    &format!("{mid:>gutter_width$}")
                                ),
                                STYLE_GUTTER.paint(self.color, "│"),
                                text
                            );
                        }
                    }
                }
                prev_line = Some(line_num);

                let Some(line_text) = file.line_text(line_num - 1) else {
                    continue;
                };

                // Print the source line.
                let _ = writeln!(
                    out,
                    " {} {} {}",
                    STYLE_GUTTER.paint(self.color, &format!("{line_num:>gutter_width$}")),
                    STYLE_GUTTER.paint(self.color, "│"),
                    line_text
                );

                // Print underlines for labels on this line.
                let labels_here = line_labels.get(&line_num).map(|v| v.as_slice()).unwrap_or(&[]);
                if labels_here.is_empty() {
                    continue;
                }

                for label in labels_here {
                    let label_start = file.line_column(label.span.span().start());
                    let label_end = file.line_column(label.span.span().end());

                    // Only render the underline on the line where the span starts
                    // (or for single-line spans).
                    if label_start.line != line_num && label_end.line != line_num {
                        continue;
                    }

                    let col_start = if label_start.line == line_num {
                        label_start.column
                    } else {
                        1
                    };

                    let col_end = if label_end.line == line_num {
                        label_end.column
                    } else {
                        line_text.len() + 1
                    };

                    let caret_width = if col_end <= col_start { 1 } else { col_end - col_start };
                    let caret_width = caret_width.max(1);

                    let (caret_char, caret_style, msg_style) = match label.style {
                        LabelStyle::Primary => {
                            let cs = match diag.severity {
                                Severity::Error => &STYLE_PRIMARY_CARET_ERROR,
                                Severity::Warning => &STYLE_PRIMARY_CARET_WARNING,
                                _ => &STYLE_NOTE,
                            };
                            ("^", cs, cs)
                        }
                        LabelStyle::Secondary => ("─", &STYLE_SECONDARY_CARET, &STYLE_LABEL_MESSAGE),
                    };

                    let padding = " ".repeat(col_start.saturating_sub(1));
                    let carets = caret_char.repeat(caret_width);

                    if label.message.is_empty() {
                        let _ = writeln!(
                            out,
                            " {} {} {}{}",
                            " ".repeat(gutter_width),
                            STYLE_GUTTER.paint(self.color, "│"),
                            padding,
                            caret_style.paint(self.color, &carets)
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            " {} {} {}{} {}",
                            " ".repeat(gutter_width),
                            STYLE_GUTTER.paint(self.color, "│"),
                            padding,
                            caret_style.paint(self.color, &carets),
                            msg_style.paint(self.color, &label.message)
                        );
                    }
                }
            }

            // Closing gutter.
            let _ = writeln!(
                out,
                " {} {}",
                " ".repeat(gutter_width),
                STYLE_GUTTER.paint(self.color, "│")
            );

            is_first_file = false;
        }
    }

    // ── Notes ───────────────────────────────────────────────────────────

    fn write_notes(&self, out: &mut String, diag: &Diagnostic) {
        for note in &diag.notes {
            let _ = writeln!(
                out,
                " {} {}",
                STYLE_NOTE.paint(self.color, "note:"),
                note
            );
        }
    }

    // ── Help hints ──────────────────────────────────────────────────────

    fn write_help(&self, out: &mut String, diag: &Diagnostic) {
        for h in &diag.help {
            let _ = writeln!(
                out,
                " {} {}",
                STYLE_HELP.paint(self.color, "help:"),
                h
            );
        }
    }
}

fn digit_count(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    let mut v = n;
    while v > 0 {
        count += 1;
        v /= 10;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Diagnostic, DiagnosticCode};
    use crate::source::SourceDatabase;

    fn test_sources() -> (SourceDatabase, crate::source::FileId) {
        let mut sources = SourceDatabase::new();
        let fid = sources.add_file(
            "main.aivi",
            "value greeting = \"hello\"\nsignal counter = 0\nvalue result = counter + greeting\n",
        );
        (sources, fid)
    }

    #[test]
    fn plain_renderer_contains_severity_and_message() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::error("type mismatch")
            .with_code(DiagnosticCode::new("hir", "type-mismatch"))
            .with_primary_label(file.source_span(53..78), "cannot add Int and Text");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(rendered.contains("error"), "missing severity: {rendered}");
        assert!(
            rendered.contains("type mismatch"),
            "missing message: {rendered}"
        );
        assert!(
            rendered.contains("hir::type-mismatch"),
            "missing code: {rendered}"
        );
        assert!(
            rendered.contains("main.aivi"),
            "missing file path: {rendered}"
        );
        assert!(
            rendered.contains("cannot add Int and Text"),
            "missing label: {rendered}"
        );
    }

    #[test]
    fn colored_renderer_contains_ansi_escapes() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::error("type mismatch")
            .with_primary_label(file.source_span(53..78), "wrong type");

        let renderer = DiagnosticRenderer::colored();
        let rendered = renderer.render(&diag, &sources);

        assert!(
            rendered.contains("\x1b["),
            "expected ANSI escapes: {rendered}"
        );
        assert!(rendered.contains(RESET), "expected ANSI reset: {rendered}");
    }

    #[test]
    fn renders_help_hints() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::error("type mismatch")
            .with_primary_label(file.source_span(53..78), "expected Text, found Int")
            .with_help("try using `toString` to convert the value");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(
            rendered.contains("help: try using `toString` to convert the value"),
            "missing help hint: {rendered}"
        );
    }

    #[test]
    fn renders_notes() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::warning("unused binding")
            .with_primary_label(file.source_span(6..14), "defined here but never used")
            .with_note("prefix with `_` to silence this warning");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(rendered.contains("warning"), "missing severity: {rendered}");
        assert!(
            rendered.contains("note: prefix with `_` to silence this warning"),
            "missing note: {rendered}"
        );
    }

    #[test]
    fn renders_secondary_labels() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::error("type mismatch")
            .with_primary_label(file.source_span(53..78), "Int + Text is not valid")
            .with_secondary_label(file.source_span(25..43), "counter is Int");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(
            rendered.contains("Int + Text is not valid"),
            "missing primary label: {rendered}"
        );
        assert!(
            rendered.contains("counter is Int"),
            "missing secondary label: {rendered}"
        );
    }

    #[test]
    fn renders_multiple_files() {
        let mut sources = SourceDatabase::new();
        let fid1 = sources.add_file("a.aivi", "value x = 1\n");
        let fid2 = sources.add_file("b.aivi", "value y = x + \"text\"\n");
        let file1 = &sources[fid1];
        let file2 = &sources[fid2];

        let diag = Diagnostic::error("type mismatch")
            .with_primary_label(file2.source_span(10..19), "cannot add Int and Text")
            .with_secondary_label(file1.source_span(6..7), "x is defined as Int here");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(rendered.contains("a.aivi"), "missing file a: {rendered}");
        assert!(rendered.contains("b.aivi"), "missing file b: {rendered}");
    }

    #[test]
    fn unicode_gutters_in_plain_mode() {
        let (sources, fid) = test_sources();
        let file = &sources[fid];
        let diag = Diagnostic::error("test").with_primary_label(file.source_span(0..5), "here");

        let renderer = DiagnosticRenderer::plain();
        let rendered = renderer.render(&diag, &sources);

        assert!(rendered.contains("╭─"), "missing box drawing: {rendered}");
        assert!(rendered.contains("│"), "missing gutter: {rendered}");
    }

    #[test]
    fn digit_count_works() {
        assert_eq!(digit_count(0), 1);
        assert_eq!(digit_count(1), 1);
        assert_eq!(digit_count(9), 1);
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(99), 2);
        assert_eq!(digit_count(100), 3);
        assert_eq!(digit_count(999), 3);
    }

    #[test]
    fn color_auto_detection() {
        // Explicit modes override everything.
        assert!(should_colorize(ColorMode::Always, false));
        assert!(!should_colorize(ColorMode::Never, true));
    }
}
