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
    let hir = aivi_query::hir_module(db, file);

    let mut diagnostics: Vec<lsp::Diagnostic> = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| convert_diagnostic(diagnostic, analysis.source.as_ref(), db, uri))
        .collect();

    diagnostics.extend(crate::type_annotations::collect_type_annotation_diagnostics(
        analysis.typed_declarations.as_ref(),
        analysis.source.as_ref(),
    ));

    // Append unused-symbol hints only when the file has no errors, to avoid
    // false positives while the user is actively editing.
    let has_errors = hir
        .diagnostics()
        .iter()
        .any(|d| d.severity == aivi_base::Severity::Error);
    if !has_errors {
        diagnostics.extend(crate::unused::collect_unused_diagnostics(
            hir.module(),
            analysis.source.as_ref(),
        ));
    }

    diagnostics
}

fn convert_diagnostic(
    d: &Diagnostic,
    source_file: &aivi_base::SourceFile,
    db: &aivi_query::RootDatabase,
    file_uri: &Url,
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

    // Convert secondary labels to LSP DiagnosticRelatedInformation entries so
    // editors can navigate to additional context spans referenced by the
    // diagnostic (e.g. a previous definition site).
    let related_information: Vec<DiagnosticRelatedInformation> = d
        .labels
        .iter()
        .filter(|l| l.style == LabelStyle::Secondary)
        .filter_map(|label| {
            let label_file_id = label.span.file();
            // Prefer looking up the URI from the database so cross-file
            // secondary labels resolve to the correct document URI.  Fall back
            // to the current file's URI when the file cannot be located.
            let matched_file = db
                .files()
                .into_iter()
                .find(|qf| qf.source(db).id() == label_file_id);

            let label_uri = matched_file
                .as_ref()
                .and_then(|qf| Url::from_file_path(qf.path(db)).ok())
                .unwrap_or_else(|| file_uri.clone());

            // Resolve the label's source file to compute the LSP range.
            let label_source = matched_file
                .map(|qf| qf.source(db))
                .unwrap_or_else(|| std::sync::Arc::new(source_file.clone()));

            let lsp_r = label_source.span_to_lsp_range(label.span.span());
            let label_range = lsp_range(lsp_r);
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri: label_uri,
                    range: label_range,
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
