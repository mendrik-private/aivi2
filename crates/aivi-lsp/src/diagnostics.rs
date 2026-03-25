use aivi_base::{Diagnostic, LabelStyle, LspRange, Severity};
use tower_lsp::lsp_types::{
    self as lsp, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Url,
};

/// Convert an aivi_base::LspRange to a tower-lsp Range.
pub fn lsp_range(r: LspRange) -> Range {
    Range {
        start: Position {
            line: r.start.line,
            character: r.start.character,
        },
        end: Position {
            line: r.end.line,
            character: r.end.character,
        },
    }
}

/// Collect all diagnostics for a file and convert to LSP format.
pub fn collect_lsp_diagnostics(
    db: &aivi_query::RootDatabase,
    file: aivi_query::SourceFile,
    uri: &Url,
) -> Vec<lsp::Diagnostic> {
    let analysis = crate::analysis::FileAnalysis::load(db, file);

    analysis
        .diagnostics
        .iter()
        .map(|diagnostic| convert_diagnostic(diagnostic, analysis.source.as_ref(), uri))
        .collect()
}

fn convert_diagnostic(
    d: &Diagnostic,
    source_file: &aivi_base::SourceFile,
    uri: &Url,
) -> lsp::Diagnostic {
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Note => DiagnosticSeverity::INFORMATION,
        Severity::Help => DiagnosticSeverity::HINT,
    };

    let range = d
        .labels
        .iter()
        .find(|l| l.style == aivi_base::LabelStyle::Primary)
        .or_else(|| d.labels.first())
        .map(|label| {
            let lsp_r = source_file.span_to_lsp_range(label.span.span());
            lsp_range(lsp_r)
        })
        .unwrap_or_else(|| Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        });

    let code = d.code.map(|c| NumberOrString::String(c.to_string()));

    let related_information: Vec<DiagnosticRelatedInformation> = d
        .labels
        .iter()
        .filter(|l| l.style != LabelStyle::Primary)
        .filter_map(|label| {
            if label.span.file() != source_file.id() {
                return None;
            }
            let lsp_r = source_file.span_to_lsp_range(label.span.span());
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range: lsp_range(lsp_r),
                },
                message: label.message.clone(),
            })
        })
        .collect();

    lsp::Diagnostic {
        range,
        severity: Some(severity),
        code,
        code_description: None,
        source: Some("aivi".to_owned()),
        message: d.message.clone(),
        related_information: if related_information.is_empty() {
            None
        } else {
            Some(related_information)
        },
        tags: None,
        data: None,
    }
}
