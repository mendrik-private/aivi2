use std::sync::Arc;

use aivi_base::LspPosition;
use aivi_hir::LspSymbolKind;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
};

use crate::state::ServerState;

pub async fn completion(
    params: CompletionParams,
    state: Arc<ServerState>,
) -> Option<CompletionResponse> {
    let uri = &params.text_document_position.text_document.uri;
    let lsp_pos = params.text_document_position.position;

    let file = *state.files.get(uri)?;
    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);

    // TODO(aivi-base): lsp_position_to_offset now returns Option<ByteIndex>; update this
    // call site to handle None (out-of-range column) instead of relying on the old
    // silent-clamping behaviour.
    let cursor = analysis.source.lsp_position_to_offset(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    })?;

    // Walk inward to the tightest symbol at the cursor position.
    let sym = analysis.tightest_symbol_at_offset(cursor)?;

    // If the tightest symbol is a record/struct/namespace, offer its children
    // as field or member completions.
    let child_completions = match sym.kind {
        LspSymbolKind::Struct | LspSymbolKind::Namespace | LspSymbolKind::Class => {
            sym.children
                .iter()
                .map(|child| {
                    let kind = lsp_symbol_kind_to_completion_kind(child.kind);
                    CompletionItem {
                        label: child.name.clone(),
                        kind: Some(kind),
                        detail: child.detail.clone(),
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>()
        }
        // For any other symbol, offer the symbol's children (e.g. function parameters,
        // enum members) as candidates.
        _ => sym
            .children
            .iter()
            .map(|child| {
                let kind = lsp_symbol_kind_to_completion_kind(child.kind);
                CompletionItem {
                    label: child.name.clone(),
                    kind: Some(kind),
                    detail: child.detail.clone(),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>(),
    };

    Some(CompletionResponse::Array(child_completions))
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
