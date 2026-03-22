use aivi_base::SourceSpan;

use crate::{Item, Module};

/// LSP symbol kinds (mirrors the LSP spec SymbolKind enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum LspSymbolKind {
    File = 1,
    Module = 2,
    Namespace = 3,
    Package = 4,
    Class = 5,
    Method = 6,
    Property = 7,
    Field = 8,
    Constructor = 9,
    Enum = 10,
    Interface = 11,
    Function = 12,
    Variable = 13,
    Constant = 14,
    String = 15,
    Number = 16,
    Boolean = 17,
    Array = 18,
    Object = 19,
    Key = 20,
    Null = 21,
    EnumMember = 22,
    Struct = 23,
    Event = 24,
    Operator = 25,
    TypeParameter = 26,
}

/// A single symbol for LSP documentSymbol / workspace/symbol responses.
#[derive(Clone, Debug)]
pub struct LspSymbol {
    pub name: String,
    pub kind: LspSymbolKind,
    /// Full span of the declaration (for the range field).
    pub span: SourceSpan,
    /// Span of just the name token (for selectionRange).
    pub selection_span: SourceSpan,
    /// Optional detail string (e.g. type annotation).
    pub detail: Option<String>,
    /// Children (e.g. class members, domain members).
    pub children: Vec<LspSymbol>,
}

/// Extract LSP symbols from a HIR module.
///
/// Returns a flat-then-nested list of symbols suitable for `textDocument/documentSymbol`.
pub fn extract_symbols(module: &Module) -> Vec<LspSymbol> {
    let mut symbols = Vec::new();

    for &id in module.root_items() {
        if let Some(item) = module.items().get(id) {
            if let Some(sym) = item_to_lsp_symbol(item) {
                symbols.push(sym);
            }
        }
    }

    symbols
}

fn item_to_lsp_symbol(item: &Item) -> Option<LspSymbol> {
    match item {
        Item::Type(t) => Some(LspSymbol {
            name: t.name.text().to_owned(),
            kind: LspSymbolKind::Struct,
            span: t.header.span,
            selection_span: t.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Value(v) => Some(LspSymbol {
            name: v.name.text().to_owned(),
            kind: LspSymbolKind::Variable,
            span: v.header.span,
            selection_span: v.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Function(f) => Some(LspSymbol {
            name: f.name.text().to_owned(),
            kind: LspSymbolKind::Function,
            span: f.header.span,
            selection_span: f.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Signal(s) => Some(LspSymbol {
            name: s.name.text().to_owned(),
            kind: LspSymbolKind::Event,
            span: s.header.span,
            selection_span: s.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Class(c) => Some(LspSymbol {
            name: c.name.text().to_owned(),
            kind: LspSymbolKind::Interface,
            span: c.header.span,
            selection_span: c.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Domain(d) => Some(LspSymbol {
            name: d.name.text().to_owned(),
            kind: LspSymbolKind::Namespace,
            span: d.header.span,
            selection_span: d.name.span(),
            detail: None,
            children: Vec::new(),
        }),
        Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::SourceProviderContract(_) => {
            None
        }
    }
}
