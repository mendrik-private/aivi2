use aivi_hir::{LspSymbol, LspSymbolKind};
use tower_lsp::lsp_types::{DocumentSymbol, Position, Range, SymbolKind};

/// Convert an aivi LspSymbolKind to LSP SymbolKind.
fn convert_symbol_kind(kind: LspSymbolKind) -> SymbolKind {
    match kind {
        LspSymbolKind::File => SymbolKind::FILE,
        LspSymbolKind::Module => SymbolKind::MODULE,
        LspSymbolKind::Namespace => SymbolKind::NAMESPACE,
        LspSymbolKind::Package => SymbolKind::PACKAGE,
        LspSymbolKind::Class => SymbolKind::CLASS,
        LspSymbolKind::Method => SymbolKind::METHOD,
        LspSymbolKind::Property => SymbolKind::PROPERTY,
        LspSymbolKind::Field => SymbolKind::FIELD,
        LspSymbolKind::Constructor => SymbolKind::CONSTRUCTOR,
        LspSymbolKind::Enum => SymbolKind::ENUM,
        LspSymbolKind::Interface => SymbolKind::INTERFACE,
        LspSymbolKind::Function => SymbolKind::FUNCTION,
        LspSymbolKind::Variable => SymbolKind::VARIABLE,
        LspSymbolKind::Constant => SymbolKind::CONSTANT,
        LspSymbolKind::String => SymbolKind::STRING,
        LspSymbolKind::Number => SymbolKind::NUMBER,
        LspSymbolKind::Boolean => SymbolKind::BOOLEAN,
        LspSymbolKind::Array => SymbolKind::ARRAY,
        LspSymbolKind::Object => SymbolKind::OBJECT,
        LspSymbolKind::Key => SymbolKind::KEY,
        LspSymbolKind::Null => SymbolKind::NULL,
        LspSymbolKind::EnumMember => SymbolKind::ENUM_MEMBER,
        LspSymbolKind::Struct => SymbolKind::STRUCT,
        LspSymbolKind::Event => SymbolKind::EVENT,
        LspSymbolKind::Operator => SymbolKind::OPERATOR,
        LspSymbolKind::TypeParameter => SymbolKind::TYPE_PARAMETER,
    }
}

/// Convert aivi LspSymbol list to LSP DocumentSymbol list.
pub fn convert_symbols(
    symbols: &[LspSymbol],
    source_file: &aivi_base::SourceFile,
) -> Vec<DocumentSymbol> {
    symbols
        .iter()
        .map(|symbol| convert_symbol(symbol, source_file))
        .collect()
}

fn convert_symbol(sym: &LspSymbol, source_file: &aivi_base::SourceFile) -> DocumentSymbol {
    let range = source_file.span_to_lsp_range(sym.span.span());
    let selection_range = source_file.span_to_lsp_range(sym.selection_span.span());

    let lsp_range = |r: aivi_base::LspRange| Range {
        start: Position {
            line: r.start.line,
            character: r.start.character,
        },
        end: Position {
            line: r.end.line,
            character: r.end.character,
        },
    };

    let children: Vec<DocumentSymbol> = sym
        .children
        .iter()
        .map(|child| convert_symbol(child, source_file))
        .collect();

    #[allow(deprecated)]
    DocumentSymbol {
        name: sym.name.clone(),
        detail: sym.detail.clone(),
        kind: convert_symbol_kind(sym.kind),
        tags: None,
        deprecated: None,
        range: lsp_range(range),
        selection_range: lsp_range(selection_range),
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    }
}
