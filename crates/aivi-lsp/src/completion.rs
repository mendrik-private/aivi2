use std::sync::Arc;

use aivi_base::LspPosition;
use aivi_hir::LspSymbolKind;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
};

use crate::{analysis::FileAnalysis, state::ServerState};

pub async fn completion(
    params: CompletionParams,
    state: Arc<ServerState>,
) -> Option<CompletionResponse> {
    let uri = &params.text_document_position.text_document.uri;
    let lsp_pos = params.text_document_position.position;

    let file = *state.files.get(uri)?;
    let current_analysis = FileAnalysis::load(&state.db, file);

    // Reject out-of-range cursor positions before returning any items.
    current_analysis.source.lsp_position_to_offset(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    })?;

    let mut items: Vec<CompletionItem> = Vec::new();

    // 1. Top-level symbols from the current file.
    collect_top_level(&current_analysis, &mut items);

    // 2. Exported symbols from all other tracked files.
    for entry in state.files.iter() {
        let other_uri = entry.key();
        if other_uri == uri {
            continue;
        }
        let &other_file = entry.value();
        let other_analysis = FileAnalysis::load(&state.db, other_file);
        collect_top_level(&other_analysis, &mut items);
    }

    // Deduplicate by label (keep first occurrence).
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.label.clone()));

    if items.is_empty() {
        None
    } else {
        Some(CompletionResponse::Array(items))
    }
}

fn collect_top_level(analysis: &FileAnalysis, out: &mut Vec<CompletionItem>) {
    for sym in analysis.symbols.iter() {
        out.push(CompletionItem {
            label: sym.name.clone(),
            kind: Some(lsp_symbol_kind_to_completion_kind(sym.kind)),
            detail: sym.detail.clone(),
            ..Default::default()
        });
    }
}

fn lsp_symbol_kind_to_completion_kind(kind: LspSymbolKind) -> CompletionItemKind {
    match kind {
        LspSymbolKind::Function | LspSymbolKind::Method => CompletionItemKind::FUNCTION,
        LspSymbolKind::Variable | LspSymbolKind::Constant => CompletionItemKind::VARIABLE,
        LspSymbolKind::Field | LspSymbolKind::Property => CompletionItemKind::FIELD,
        LspSymbolKind::Enum => CompletionItemKind::ENUM,
        LspSymbolKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
        LspSymbolKind::Struct => CompletionItemKind::STRUCT,
        LspSymbolKind::Class | LspSymbolKind::Interface => CompletionItemKind::CLASS,
        LspSymbolKind::Module | LspSymbolKind::Namespace | LspSymbolKind::Package => {
            CompletionItemKind::MODULE
        }
        LspSymbolKind::Constructor => CompletionItemKind::CONSTRUCTOR,
        LspSymbolKind::TypeParameter => CompletionItemKind::TYPE_PARAMETER,
        LspSymbolKind::Operator => CompletionItemKind::OPERATOR,
        _ => CompletionItemKind::TEXT,
    }
}
