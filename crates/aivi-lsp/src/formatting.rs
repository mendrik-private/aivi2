use tower_lsp::lsp_types::TextEdit;

/// Format a document and return LSP text edits.
pub fn format_document(
    db: &aivi_query::RootDatabase,
    file: aivi_query::SourceFile,
) -> Option<Vec<TextEdit>> {
    let parsed = aivi_query::parsed_file(db, file);
    let source = parsed.source_arc();
    let formatted = aivi_query::format_file(db, file)?;

    if formatted == source.text() {
        return Some(Vec::new());
    }

    Some(vec![TextEdit {
        range: crate::diagnostics::lsp_range(source.span_to_lsp_range(source.full_span().span())),
        new_text: formatted,
    }])
}
