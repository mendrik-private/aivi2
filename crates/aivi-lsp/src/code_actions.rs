use std::sync::Arc;

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse,
    NumberOrString, Position, Range, TextEdit, WorkspaceEdit,
};

use crate::{analysis::FileAnalysis, state::ServerState};

/// Produce code actions for the requested range.
///
/// Currently emits a "Remove unused symbol" quickfix for every
/// `aivi/unused-symbol` diagnostic that overlaps the request range.
pub fn code_actions(
    params: CodeActionParams,
    state: Arc<ServerState>,
) -> Option<CodeActionResponse> {
    let uri = &params.text_document.uri;
    let file = *state.files.get(uri)?;
    let analysis = FileAnalysis::load(&state.db, file);
    let hir = aivi_query::hir_module(&state.db, file);

    // Generate the unused-symbol diagnostics from the LSP layer.
    let unused_diags = crate::unused::collect_unused_diagnostics(hir.module(), &analysis.source);

    let request_range = params.range;
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();

    actions.extend(crate::type_annotations::build_type_annotation_code_actions(
        uri,
        analysis.typed_declarations.as_ref(),
        analysis.source.as_ref(),
        request_range,
    ));

    for diag in &unused_diags {
        if diag.code != Some(NumberOrString::String("aivi/unused-symbol".to_owned())) {
            continue;
        }

        // Include this action if the diagnostic range overlaps the requested range.
        if !ranges_overlap(diag.range, request_range) {
            continue;
        }

        // Build a TextEdit that deletes the entire line containing the symbol.
        let Some(line_edit) = delete_line_edit(&analysis.source, diag.range) else {
            continue;
        };

        let mut changes = std::collections::HashMap::new();
        changes.insert(uri.clone(), vec![line_edit]);

        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Remove unused symbol".to_owned(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            ..Default::default()
        }));
    }

    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

fn ranges_overlap(a: Range, b: Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Build a `TextEdit` that removes the entire source line (including newline)
/// that contains the primary range of a diagnostic.
fn delete_line_edit(source: &aivi_base::SourceFile, range: Range) -> Option<TextEdit> {
    let text = source.text();
    let line = range.start.line as usize;
    let line_start = line_start_byte(text, line)?;
    let line_end = line_end_byte(text, line_start);

    Some(TextEdit {
        range: Range {
            start: Position {
                line: range.start.line,
                character: 0,
            },
            end: line_end_position(text, line_start, line_end, range.start.line),
        },
        new_text: String::new(),
    })
}

fn line_start_byte(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    let mut current_line = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' {
            current_line += 1;
            if current_line == line {
                return Some(byte_idx + 1);
            }
        }
    }
    None
}

fn line_end_byte(text: &str, line_start: usize) -> usize {
    let rest = &text[line_start..];
    match rest.find('\n') {
        Some(rel) => line_start + rel + 1,
        None => text.len(),
    }
}

fn line_end_position(text: &str, line_start: usize, line_end: usize, line: u32) -> Position {
    // If the line ends with a newline, the deletion covers up to the start of
    // the next line (so the entire line including the newline is removed).
    // If it is the last line without a trailing newline, end at the last char.
    let end_byte = line_end;
    let suffix = &text[line_start..end_byte];
    let chars = suffix.encode_utf16().count() as u32;
    if text[..line_end].ends_with('\n') {
        // Advance to start of next line
        Position {
            line: line + 1,
            character: 0,
        }
    } else {
        Position {
            line,
            character: chars,
        }
    }
}
