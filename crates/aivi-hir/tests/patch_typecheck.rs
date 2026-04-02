use aivi_base::SourceDatabase;
use aivi_hir::{lower_module, typecheck_module};
use aivi_syntax::parse_module;

fn typecheck_text(path: &str, text: &str) -> aivi_hir::TypeCheckReport {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "typecheck input should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let lowered = lower_module(&parsed.module);
    assert!(
        !lowered.has_errors(),
        "typecheck input should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    typecheck_module(lowered.module())
}

#[test]
fn accepts_patch_apply_over_list_predicates() {
    let report = typecheck_text(
        "patch-list-predicate.aivi",
        "type Item = { active: Bool, price: Int }\n\
         value items:List Item = [{ active: True, price: 1 }, { active: False, price: 2 }]\n\
         value updated:List Item = items <| { [.active].price: 3 }\n",
    );
    assert!(
        report.is_ok(),
        "expected list predicate patches to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn accepts_patch_apply_over_map_entry_predicates() {
    let report = typecheck_text(
        "patch-map-predicate.aivi",
        "type User = { name: Text }\n\
         value users:Map Text User = Map { \"id-1\": { name: \"Ada\" }, \"id-2\": { name: \"Grace\" } }\n\
         value updated:Map Text User = users <| { [.key == \"id-1\"].name: \"Ava\" }\n",
    );
    assert!(
        report.is_ok(),
        "expected map predicate patches to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn accepts_patch_literals_with_bare_record_field_selectors() {
    let report = typecheck_text(
        "patch-literal-bare-field.aivi",
        "type User = { name: Text }\n\
         value rename:(User -> User) = patch { name: \"Grace\" }\n",
    );
    assert!(
        report.is_ok(),
        "expected patch literals with bare field selectors to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn accepts_patch_literals_with_dotted_record_field_selectors() {
    let report = typecheck_text(
        "patch-literal-dotted-field.aivi",
        "type User = { name: Text }\n\
         value rename:(User -> User) = patch { .name: \"Grace\" }\n",
    );
    assert!(
        report.is_ok(),
        "expected patch literals with dotted field selectors to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn accepts_patch_constructor_focus_for_single_payload_constructors() {
    let report = typecheck_text(
        "patch-constructor-focus.aivi",
        "type Point = { count: Int }\n\
         value maybePoint:Option Point = Some { count: 1 }\n\
         value updated:Option Point = maybePoint <| { Some.count: 2 }\n",
    );
    assert!(
        report.is_ok(),
        "expected single-payload constructor focus patches to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn reports_patch_remove_as_unsupported() {
    let report = typecheck_text(
        "patch-remove-unsupported.aivi",
        "type User = { name: Text }\n\
         value user:User = { name: \"Ada\" }\n\
         value updated = user <| { .name: - }\n",
    );
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(aivi_hir::codes::UNSUPPORTED_PATCH_REMOVE)
        }),
        "expected unsupported patch removal diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}
