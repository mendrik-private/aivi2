use aivi_base::SourceSpan;

use crate::{DomainMemberKind, Item, Module, TypeId, TypeItemBody, TypeKind};

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
            if let Some(sym) = item_to_lsp_symbol(item, module) {
                symbols.push(sym);
            }
        }
    }

    symbols
}

fn format_type(module: &Module, id: TypeId) -> String {
    format_type_depth(module, id, 0)
}

fn format_type_depth(module: &Module, id: TypeId, depth: u8) -> String {
    if depth > 8 {
        return "..".to_owned();
    }
    let Some(node) = module.types().get(id) else {
        return "?".to_owned();
    };
    match &node.kind {
        TypeKind::Name(type_ref) => type_ref.path.to_string(),
        TypeKind::Tuple(ids) => {
            let parts: Vec<_> = ids
                .iter()
                .map(|&id| format_type_depth(module, id, depth + 1))
                .collect();
            format!("({})", parts.join(", "))
        }
        TypeKind::Record(fields) => {
            let parts: Vec<_> = fields
                .iter()
                .map(|f| {
                    format!(
                        "{}: {}",
                        f.label.text(),
                        format_type_depth(module, f.ty, depth + 1)
                    )
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        TypeKind::RecordTransform { transform, source } => {
            let source_str = format_type_depth(module, *source, depth + 1);
            match transform {
                crate::RecordRowTransform::Pick(labels) => {
                    format!("Pick ({}) {}", format_symbol_labels(labels), source_str)
                }
                crate::RecordRowTransform::Omit(labels) => {
                    format!("Omit ({}) {}", format_symbol_labels(labels), source_str)
                }
                crate::RecordRowTransform::Optional(labels) => {
                    format!("Optional ({}) {}", format_symbol_labels(labels), source_str)
                }
                crate::RecordRowTransform::Required(labels) => {
                    format!("Required ({}) {}", format_symbol_labels(labels), source_str)
                }
                crate::RecordRowTransform::Defaulted(labels) => {
                    format!(
                        "Defaulted ({}) {}",
                        format_symbol_labels(labels),
                        source_str
                    )
                }
                crate::RecordRowTransform::Rename(renames) => format!(
                    "Rename {{ {} }} {}",
                    renames
                        .iter()
                        .map(|rename| format!("{}: {}", rename.from.text(), rename.to.text()))
                        .collect::<Vec<_>>()
                        .join(", "),
                    source_str
                ),
            }
        }
        TypeKind::Arrow { parameter, result } => {
            let param_str = format_type_depth(module, *parameter, depth + 1);
            let result_str = format_type_depth(module, *result, depth + 1);
            format!("{} -> {}", param_str, result_str)
        }
        TypeKind::Apply { callee, arguments } => {
            let callee_str = format_type_depth(module, *callee, depth + 1);
            let args: Vec<_> = arguments
                .iter()
                .map(|&id| format_type_depth(module, id, depth + 1))
                .collect();
            format!("{} {}", callee_str, args.join(" "))
        }
    }
}

fn format_symbol_labels(labels: &[crate::Name]) -> String {
    labels
        .iter()
        .map(|label| label.text())
        .collect::<Vec<_>>()
        .join(", ")
}

fn item_to_lsp_symbol(item: &Item, module: &Module) -> Option<LspSymbol> {
    match item {
        Item::Type(t) => {
            let (detail, children) = match &t.body {
                TypeItemBody::Alias(ty_id) => (Some(format_type(module, *ty_id)), Vec::new()),
                TypeItemBody::Sum(variants) => {
                    let children = variants
                        .iter()
                        .map(|v| {
                            let detail = if v.fields.is_empty() {
                                None
                            } else {
                                let parts: Vec<_> =
                                    v.fields.iter().map(|&f| format_type(module, f)).collect();
                                Some(format!("({})", parts.join(", ")))
                            };
                            LspSymbol {
                                name: v.name.text().to_owned(),
                                kind: LspSymbolKind::EnumMember,
                                span: v.span,
                                selection_span: v.name.span(),
                                detail,
                                children: Vec::new(),
                            }
                        })
                        .collect();
                    (None, children)
                }
            };
            Some(LspSymbol {
                name: t.name.text().to_owned(),
                kind: LspSymbolKind::Struct,
                span: t.header.span,
                selection_span: t.name.span(),
                detail,
                children,
            })
        }
        Item::Value(v) => Some(LspSymbol {
            name: v.name.text().to_owned(),
            kind: LspSymbolKind::Variable,
            span: v.header.span,
            selection_span: v.name.span(),
            detail: v.annotation.map(|id| format_type(module, id)),
            children: Vec::new(),
        }),
        Item::Function(f) => {
            let params_str = if f.parameters.is_empty() {
                String::new()
            } else {
                let parts: Vec<_> = f
                    .parameters
                    .iter()
                    .map(|p| {
                        let name = module
                            .bindings()
                            .get(p.binding)
                            .map(|b| b.name.text().to_owned())
                            .unwrap_or_else(|| "_".to_owned());
                        match p.annotation {
                            Some(ty_id) => format!("{}: {}", name, format_type(module, ty_id)),
                            None => name,
                        }
                    })
                    .collect();
                format!("({})", parts.join(", "))
            };
            let return_str = f.annotation.map(|id| format_type(module, id));
            let detail = match (params_str.is_empty(), return_str) {
                (true, None) => None,
                (true, Some(ret)) => Some(ret),
                (false, None) => Some(params_str),
                (false, Some(ret)) => Some(format!("{} -> {}", params_str, ret)),
            };
            let param_children: Vec<LspSymbol> = f
                .parameters
                .iter()
                .filter_map(|p| {
                    let binding = module.bindings().get(p.binding)?;
                    Some(LspSymbol {
                        name: binding.name.text().to_owned(),
                        kind: LspSymbolKind::Variable,
                        span: p.span,
                        selection_span: binding.span,
                        detail: p.annotation.map(|id| format_type(module, id)),
                        children: Vec::new(),
                    })
                })
                .collect();
            Some(LspSymbol {
                name: f.name.text().to_owned(),
                kind: LspSymbolKind::Function,
                span: f.header.span,
                selection_span: f.name.span(),
                detail,
                children: param_children,
            })
        }
        Item::Signal(s) => Some(LspSymbol {
            name: s.name.text().to_owned(),
            kind: LspSymbolKind::Event,
            span: s.header.span,
            selection_span: s.name.span(),
            detail: s.annotation.map(|id| format_type(module, id)),
            children: s
                .reactive_updates
                .iter()
                .enumerate()
                .map(|(index, update)| LspSymbol {
                    name: format!("when #{}", index + 1),
                    kind: LspSymbolKind::Event,
                    span: update.span,
                    selection_span: update.keyword_span,
                    detail: Some("reactive update".to_owned()),
                    children: Vec::new(),
                })
                .collect(),
        }),
        Item::Class(c) => {
            let children = c
                .members
                .iter()
                .map(|m| LspSymbol {
                    name: m.name.text().to_owned(),
                    kind: LspSymbolKind::Method,
                    span: m.span,
                    selection_span: m.name.span(),
                    detail: Some(format_type(module, m.annotation)),
                    children: Vec::new(),
                })
                .collect();
            Some(LspSymbol {
                name: c.name.text().to_owned(),
                kind: LspSymbolKind::Interface,
                span: c.header.span,
                selection_span: c.name.span(),
                detail: None,
                children,
            })
        }
        Item::Domain(d) => {
            let children = d
                .members
                .iter()
                .map(|m| {
                    let kind = match m.kind {
                        DomainMemberKind::Method => LspSymbolKind::Method,
                        DomainMemberKind::Operator => LspSymbolKind::Operator,
                        DomainMemberKind::Literal => LspSymbolKind::EnumMember,
                    };
                    LspSymbol {
                        name: m.name.text().to_owned(),
                        kind,
                        span: m.span,
                        selection_span: m.name.span(),
                        detail: Some(format_type(module, m.annotation)),
                        children: Vec::new(),
                    }
                })
                .collect();
            Some(LspSymbol {
                name: d.name.text().to_owned(),
                kind: LspSymbolKind::Namespace,
                span: d.header.span,
                selection_span: d.name.span(),
                detail: None,
                children,
            })
        }
        Item::Instance(_) | Item::Use(_) | Item::Export(_) | Item::SourceProviderContract(_) => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{LspSymbolKind, extract_symbols};

    fn lower_symbols(input: &str) -> Vec<super::LspSymbol> {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file("symbols.aivi", input.to_owned());
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "symbol test input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = crate::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "symbol test input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        extract_symbols(lowered.module())
    }

    #[test]
    fn signal_symbols_include_reactive_update_children() {
        let symbols = lower_symbols(
            r#"signal total : Signal Int
signal ready : Signal Bool

when ready => total <- 1
"#,
        );

        let total = symbols
            .iter()
            .find(|symbol| symbol.name == "total")
            .expect("expected total signal symbol");
        assert_eq!(total.kind, LspSymbolKind::Event);
        assert_eq!(total.children.len(), 1);
        assert_eq!(total.children[0].name, "when #1");
        assert_eq!(total.children[0].kind, LspSymbolKind::Event);
    }
}
