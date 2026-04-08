use std::collections::HashSet;

use aivi_base::{Diagnostic, DiagnosticCode};
use aivi_hir::{
    DecoratorPayload, ExprKind, Item, ItemId, Module, ResolutionState, TermResolution, TypeKind,
    TypeResolution,
};
use tower_lsp::lsp_types::{self as lsp, DiagnosticSeverity, DiagnosticTag, NumberOrString};

use crate::diagnostics::lsp_range;

/// Collect LSP diagnostics for symbols that are defined but never referenced
/// within the module and are not explicitly exported.
///
/// Unused symbols are emitted as `Hint` severity diagnostics tagged with
/// `DiagnosticTag::UNNECESSARY` so that editors like VSCode dim them.
pub fn collect_unused_diagnostics(
    module: &Module,
    source: &aivi_base::SourceFile,
) -> Vec<lsp::Diagnostic> {
    let referenced = collect_referenced_items(module);
    let exported = collect_exported_items(module);

    let mut diagnostics = Vec::new();

    for item_id in module.root_items() {
        // Skip items that are referenced or exported.
        if referenced.contains(item_id)
            || exported.contains(item_id)
            || skip_unused_diagnostic(module, *item_id)
        {
            continue;
        }

        let Some((name_text, name_span)) = item_name_and_span(module, *item_id) else {
            continue;
        };

        let lsp_r = source.span_to_lsp_range(name_span.span());
        diagnostics.push(lsp::Diagnostic {
            range: lsp_range(lsp_r),
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(NumberOrString::String("aivi/unused-symbol".to_owned())),
            code_description: None,
            source: Some("aivi".to_owned()),
            message: format!("`{name_text}` is defined but never used"),
            related_information: None,
            tags: Some(vec![DiagnosticTag::UNNECESSARY]),
            data: None,
        });
    }

    diagnostics
}

/// Collect unused-symbol warnings as native [`aivi_base::Diagnostic`] items so
/// that CLI tools can render them without depending on LSP types.
///
/// Only called when the module has no HIR errors, matching the LSP behaviour.
pub fn collect_unused_native_diagnostics(
    module: &Module,
    source: &aivi_base::SourceFile,
) -> Vec<Diagnostic> {
    let referenced = collect_referenced_items(module);
    let exported = collect_exported_items(module);

    let mut diagnostics = Vec::new();

    for item_id in module.root_items() {
        if referenced.contains(item_id)
            || exported.contains(item_id)
            || skip_unused_diagnostic(module, *item_id)
        {
            continue;
        }

        let Some((name_text, name_span)) = item_name_and_span(module, *item_id) else {
            continue;
        };
        let _ = source; // span is already file-scoped; kept for API symmetry
        diagnostics.push(
            Diagnostic::warning(format!("`{name_text}` is defined but never used"))
                .with_code(DiagnosticCode::new("aivi", "unused-symbol"))
                .with_primary_label(name_span, "defined here"),
        );
    }

    diagnostics
}

/// Collect all ItemIds directly referenced from expressions and type nodes
/// throughout the module, including signal dependency lists and instance class
/// references.
fn collect_referenced_items(module: &Module) -> HashSet<ItemId> {
    let mut referenced = HashSet::new();

    // Scan all expressions for term-level item references.
    for (_, expr) in module.exprs().iter() {
        if let ExprKind::Name(reference) = &expr.kind {
            if let ResolutionState::Resolved(TermResolution::Item(id)) = reference.resolution {
                referenced.insert(id);
            }
        }
    }

    // Scan all type nodes for type-level item references.
    for (_, ty) in module.types().iter() {
        if let TypeKind::Name(reference) = &ty.kind {
            if let ResolutionState::Resolved(TypeResolution::Item(id)) = reference.resolution {
                referenced.insert(id);
            }
        }
    }

    // Walk all items for structural item references that don't appear in the
    // expression or type arenas directly (e.g. signal dependency lists,
    // instance class references).
    for (_, item) in module.items().iter() {
        match item {
            Item::Signal(signal) => {
                referenced.extend(signal.signal_dependencies.iter().copied());
                if let Some(source_meta) = &signal.source_metadata {
                    referenced.extend(source_meta.signal_dependencies.iter().copied());
                }
                for update in &signal.reactive_updates {
                    if let Some(trigger) = update.trigger_source {
                        referenced.insert(trigger);
                    }
                }
            }
            Item::Instance(instance) => {
                // The class being instantiated is referenced by the instance.
                if let ResolutionState::Resolved(TypeResolution::Item(id)) =
                    instance.class.resolution
                {
                    referenced.insert(id);
                }
            }
            _ => {}
        }
    }

    referenced
}

/// Collect all ItemIds that are explicitly exported from the module.
fn collect_exported_items(module: &Module) -> HashSet<ItemId> {
    let mut exported = HashSet::new();

    for item_id in module.root_items() {
        if let Item::Export(export) = &module.items()[*item_id] {
            if let ResolutionState::Resolved(aivi_hir::ExportResolution::Item(id)) =
                export.resolution
            {
                exported.insert(id);
            }
        }
    }

    exported
}

fn skip_unused_diagnostic(module: &Module, item_id: ItemId) -> bool {
    module.items()[item_id]
        .decorators()
        .iter()
        .any(|decorator_id| {
            matches!(
                module.decorators()[*decorator_id].payload,
                DecoratorPayload::Test(_)
            )
        })
}

/// Extract the name text and name span for an item, if the item kind has a
/// user-visible name. Returns `None` for structural items (export, use,
/// instance, source provider contract) that have no meaningful "unused" state.
fn item_name_and_span(module: &Module, item_id: ItemId) -> Option<(&str, aivi_base::SourceSpan)> {
    match &module.items()[item_id] {
        Item::Value(item) => Some((item.name.text(), item.name.span())),
        Item::Function(item) => Some((item.name.text(), item.name.span())),
        Item::Signal(item) => Some((item.name.text(), item.name.span())),
        Item::Type(item) => Some((item.name.text(), item.name.span())),
        Item::Domain(item) => Some((item.name.text(), item.name.span())),
        Item::Class(item) => Some((item.name.text(), item.name.span())),
        // Export, Use, Instance, SourceProviderContract, Hoist: skip unused reporting.
        Item::Export(_)
        | Item::Use(_)
        | Item::Instance(_)
        | Item::SourceProviderContract(_)
        | Item::Hoist(_) => None,
    }
}
