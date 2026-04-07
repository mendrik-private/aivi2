use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{
    Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Position, Range,
};

use crate::state::ServerState;

pub async fn hover(params: HoverParams, state: Arc<ServerState>) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let lsp_pos = params.text_document_position_params.position;

    let file = *state.files.get(uri)?;
    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);
    let cursor = LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    };

    if let Some(declaration) = analysis.typed_declaration_at_lsp_position(cursor) {
        return Some(hover_for_typed_declaration(declaration, &analysis.source));
    }

    // Fallback: cursor may be on a navigation reference site (not a declaration symbol).
    // Resolve to the declaration and retrieve its type detail.
    let navigation = crate::navigation::NavigationAnalysis::load(&state.db, file);
    if let crate::navigation::NavigationLookup::Targets(targets) =
        navigation.definition_targets_at_lsp_position(&state.db, cursor)
    {
        for target in &targets {
            let target_analysis = crate::analysis::FileAnalysis::load(&state.db, target.file());
            if let Some(declaration) = target_analysis.typed_declaration_for_name_span(target.span) {
                return Some(hover_for_typed_declaration(
                    declaration,
                    &target_analysis.source,
                ));
            }
            if let Some(decl_sym) = target.find_symbol_at_target(&state.db) {
                return Some(hover_for_symbol(&decl_sym, &target_analysis.source));
            }
        }
    }

    if let Some(sym) = analysis.tightest_symbol_at_lsp_position(cursor) {
        return Some(hover_for_symbol(sym, &analysis.source));
    }

    None
}

fn hover_for_typed_declaration(
    declaration: &crate::type_annotations::TypedDeclarationSummary,
    source: &aivi_base::SourceFile,
) -> Hover {
    let kind_label = typed_kind_label(declaration.kind);
    let header = match (
        declaration.inferred_type.as_deref(),
        declaration.declared_type.as_deref(),
        declaration.annotation_matches_inferred,
    ) {
        (Some(inferred), Some(declared), Some(false)) => format!(
            "```aivi\n{} {} : {}\n```\n\nDeclared type: `{}`",
            kind_label, declaration.name, inferred, declared
        ),
        (Some(inferred), _, _) => {
            format!("```aivi\n{} {} : {}\n```", kind_label, declaration.name, inferred)
        }
        (None, Some(declared), _) => {
            format!("```aivi\n{} {} : {}\n```", kind_label, declaration.name, declared)
        }
        (None, None, _) => format!("```aivi\n{} {}\n```", kind_label, declaration.name),
    };

    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: header,
        }),
        range: Some(range_for_span(source, declaration.name_span)),
    }
}

fn hover_for_symbol(sym: &aivi_hir::LspSymbol, source: &aivi_base::SourceFile) -> Hover {
    let kind_label = kind_label(sym.kind);
    let header = if let Some(detail) = &sym.detail {
        format!("{} {} : {}", kind_label, sym.name, detail)
    } else {
        format!("{} {}", kind_label, sym.name)
    };

    let name_range = source.span_to_lsp_range(sym.selection_span.span());
    let range = Range {
        start: Position {
            line: name_range.start.line,
            character: name_range.start.character,
        },
        end: Position {
            line: name_range.end.line,
            character: name_range.end.character,
        },
    };

    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```aivi\n{}\n```", header),
        }),
        range: Some(range),
    }
}

fn range_for_span(source: &aivi_base::SourceFile, span: aivi_base::SourceSpan) -> Range {
    let name_range = source.span_to_lsp_range(span.span());
    Range {
        start: Position {
            line: name_range.start.line,
            character: name_range.start.character,
        },
        end: Position {
            line: name_range.end.line,
            character: name_range.end.character,
        },
    }
}

pub fn kind_label(kind: aivi_hir::LspSymbolKind) -> &'static str {
    match kind {
        aivi_hir::LspSymbolKind::Function => "func",
        aivi_hir::LspSymbolKind::Variable => "value",
        aivi_hir::LspSymbolKind::Event => "signal",
        aivi_hir::LspSymbolKind::Struct => "type",
        aivi_hir::LspSymbolKind::Interface => "class",
        aivi_hir::LspSymbolKind::Namespace => "domain",
        _ => "symbol",
    }
}

fn typed_kind_label(kind: aivi_hir::TypedDeclarationKind) -> &'static str {
    match kind {
        aivi_hir::TypedDeclarationKind::Function => "func",
        aivi_hir::TypedDeclarationKind::Value => "value",
        aivi_hir::TypedDeclarationKind::Signal => "signal",
    }
}
