use aivi_syntax::Formatter;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

/// Format a document and return LSP text edits.
pub fn format_document(text: &str, path: &std::path::Path) -> Option<Vec<TextEdit>> {
    let mut source_db = aivi_base::SourceDatabase::new();
    let file_id = source_db.add_file(path.to_path_buf(), text.to_owned());
    let source_file = &source_db[file_id];
    let parsed = aivi_syntax::parse_module(source_file);
    let formatter = Formatter;
    let formatted = formatter.format(&parsed.module);

    if formatted == text {
        return Some(Vec::new());
    }

    // Return a single edit replacing the entire document.
    let line_count = text.lines().count() as u32;
    let last_line_len = text.lines().last().map(|l| l.len() as u32).unwrap_or(0);

    Some(vec![TextEdit {
        range: Range {
            start: Position { line: 0, character: 0 },
            end: Position {
                line: line_count,
                character: last_line_len,
            },
        },
        new_text: formatted,
    }])
}
