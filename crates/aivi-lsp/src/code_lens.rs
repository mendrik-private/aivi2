use aivi_hir::{DecoratorPayload, Item, ItemId, Module};
use tower_lsp::lsp_types::{self as lsp, CodeLens, Command, Range};

use crate::diagnostics::lsp_range;

/// Collect code lens entries for all `@test` decorated items in the module.
///
/// Each test item gets a ▶ "Run test" lens above it that triggers the
/// `aivi.runTest` command with the file URI and the test value's name.
pub fn collect_code_lenses(
    module: &Module,
    source: &aivi_base::SourceFile,
    uri: &lsp::Url,
) -> Vec<CodeLens> {
    let mut lenses = Vec::new();

    for item_id in module.root_items() {
        if !item_has_test_decorator(module, *item_id) {
            continue;
        }

        let Some((name_text, name_span)) = item_name_and_span(module, *item_id) else {
            continue;
        };

        let lsp_r = source.span_to_lsp_range(name_span.span());
        let range: Range = lsp_range(lsp_r);

        lenses.push(CodeLens {
            range,
            command: Some(Command {
                title: "▶ Run test".to_owned(),
                command: "aivi.runTest".to_owned(),
                arguments: Some(vec![
                    serde_json::Value::String(uri.to_string()),
                    serde_json::Value::String(name_text.to_owned()),
                ]),
            }),
            data: None,
        });
    }

    lenses
}

fn item_has_test_decorator(module: &Module, item_id: ItemId) -> bool {
    let item = &module.items()[item_id];
    let decorator_ids = item.decorators();
    decorator_ids.iter().any(|&dec_id| {
        matches!(
            module.decorators()[dec_id].payload,
            DecoratorPayload::Test(_)
        )
    })
}

fn item_name_and_span(
    module: &Module,
    item_id: ItemId,
) -> Option<(&str, aivi_base::SourceSpan)> {
    match &module.items()[item_id] {
        Item::Value(item) => Some((item.name.text(), item.name.span())),
        Item::Function(item) => Some((item.name.text(), item.name.span())),
        Item::Signal(item) => Some((item.name.text(), item.name.span())),
        Item::Type(item) => Some((item.name.text(), item.name.span())),
        _ => None,
    }
}

