use std::{path::PathBuf, sync::Arc};

use aivi_core::{IncludedItems, RuntimeFragmentSpec};
use aivi_hir::GeneralExprOutcome;
use aivi_query::{
    RootDatabase, SourceFile, hir_module, reachable_workspace_hir_modules,
    runtime_fragment_backend_fingerprint, runtime_fragment_backend_unit,
    whole_program_backend_fingerprint, whole_program_backend_fingerprint_with_items,
    whole_program_backend_unit, whole_program_backend_unit_with_items,
};

fn first_general_expr_fragment(module: &aivi_hir::Module) -> RuntimeFragmentSpec {
    let report = aivi_hir::elaborate_general_expressions(module)
        .into_items()
        .into_iter()
        .next()
        .expect("fixture should expose at least one general-expression body");
    let body = match report.outcome {
        GeneralExprOutcome::Lowered(body) => body,
        GeneralExprOutcome::Blocked(blocked) => {
            panic!("general-expression fragment should elaborate cleanly: {blocked:?}")
        }
    };
    RuntimeFragmentSpec {
        name: format!("fragment_{}", report.owner.as_raw()).into_boxed_str(),
        owner: report.owner,
        body_expr: report.body_expr,
        parameters: report.parameters,
        body,
    }
}

fn open_workspace_math_program(
    db: &RootDatabase,
    increment: i32,
) -> (SourceFile, SourceFile, PathBuf) {
    let workspace_root = PathBuf::from("workspace");
    let dependency_path = workspace_root.join("shared/math.aivi");
    let dependency_text =
        format!("type Int -> Int\nfunc inc = x =>\n    x + {increment}\n\nexport (inc)\n");
    let dependency = SourceFile::new(db, dependency_path.clone(), dependency_text);
    let main = SourceFile::new(
        db,
        workspace_root.join("main.aivi"),
        "use shared.math (inc)\n\nvalue answer = inc 41\n".to_owned(),
    );
    (main, dependency, dependency_path)
}

#[test]
fn whole_program_backend_unit_reuses_cached_result_until_entry_text_changes() {
    let db = RootDatabase::new();
    let file = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "value answer = 42".to_owned(),
    );

    let first = whole_program_backend_unit(&db, file)
        .expect("simple whole-program lowering should produce a backend unit");
    let second = whole_program_backend_unit(&db, file)
        .expect("unchanged whole-program lowering should reuse the cached backend unit");

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        whole_program_backend_fingerprint(&db, file).expect("fingerprint query should succeed"),
        first.fingerprint()
    );

    assert!(file.set_text(&db, "value answer = 43".to_owned()));

    let third = whole_program_backend_unit(&db, file)
        .expect("updated whole-program lowering should still produce a backend unit");

    assert!(!Arc::ptr_eq(&first, &third));
    assert_ne!(first.fingerprint(), third.fingerprint());
}

#[test]
fn whole_program_backend_unit_with_items_distinguishes_entry_item_selection() {
    let db = RootDatabase::new();
    let file = SourceFile::new(
        &db,
        PathBuf::from("selection.aivi"),
        "value answer = 42\nvalue extra = 7\n".to_owned(),
    );
    let hir = hir_module(&db, file);
    let root_items = hir.module().root_items().to_vec();
    assert!(
        root_items.len() >= 2,
        "selection fixture should expose at least two root items"
    );

    let single_item = [root_items[0]].into_iter().collect::<IncludedItems>();
    let all_items = root_items.iter().copied().collect::<IncludedItems>();

    let single_first = whole_program_backend_unit_with_items(&db, file, &single_item)
        .expect("selected whole-program lowering should succeed");
    let single_second = whole_program_backend_unit_with_items(&db, file, &single_item)
        .expect("same item selection should reuse the cached backend unit");
    let all = whole_program_backend_unit_with_items(&db, file, &all_items)
        .expect("full item selection should succeed");

    assert!(Arc::ptr_eq(&single_first, &single_second));
    assert_ne!(single_first.fingerprint(), all.fingerprint());
    assert_eq!(
        whole_program_backend_fingerprint_with_items(&db, file, &single_item)
            .expect("selection-specific fingerprint query should succeed"),
        single_first.fingerprint()
    );
}

#[test]
fn reachable_workspace_hir_modules_follow_project_imports() {
    let db = RootDatabase::new();
    let (main, dependency, dependency_path) = open_workspace_math_program(&db, 1);
    let modules = reachable_workspace_hir_modules(&db, main);

    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].name(), "shared.math");
    assert_eq!(
        modules[0].file(),
        db.file_at_path(&dependency_path)
            .expect("reachable workspace module should already be registered")
    );
    assert_eq!(modules[0].file(), dependency);

    let unit = whole_program_backend_unit(&db, main)
        .expect("workspace-aware whole-program lowering should succeed");
    assert_eq!(unit.workspace_modules().len(), 1);
    assert_eq!(unit.workspace_modules()[0].name(), "shared.math");
}

#[test]
fn whole_program_backend_unit_invalidates_when_workspace_dependency_changes() {
    let db = RootDatabase::new();
    let (main, dependency, _) = open_workspace_math_program(&db, 1);
    let first = whole_program_backend_unit(&db, main)
        .expect("workspace-aware whole-program lowering should succeed");

    let original = dependency.text(&db);
    let updated = original.replace("x + 1", "x + 2");
    assert_ne!(
        original, updated,
        "workspace dependency should include the original increment body"
    );
    assert!(dependency.set_text(&db, updated));

    let second = whole_program_backend_unit(&db, main)
        .expect("workspace-dependent lowering should rebuild after dependency changes");

    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.fingerprint(), second.fingerprint());
}

#[test]
fn runtime_fragment_backend_unit_reuses_cached_result_until_fragment_changes() {
    let db = RootDatabase::new();
    let file = SourceFile::new(
        &db,
        PathBuf::from("fragment.aivi"),
        "type Int -> Int\nfunc addOne = x =>\n    x + 1\n".to_owned(),
    );

    let hir = hir_module(&db, file);
    let first_fragment = first_general_expr_fragment(hir.module());
    let first = runtime_fragment_backend_unit(&db, file, &first_fragment)
        .expect("runtime fragment lowering should produce a backend unit");
    let second = runtime_fragment_backend_unit(&db, file, &first_fragment)
        .expect("unchanged runtime fragment lowering should reuse the cached backend unit");

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        runtime_fragment_backend_fingerprint(&db, file, &first_fragment)
            .expect("runtime fragment fingerprint query should succeed"),
        first.fingerprint()
    );

    assert!(file.set_text(
        &db,
        "type Int -> Int\nfunc addOne = x =>\n    x + 2\n".to_owned(),
    ));

    let updated_hir = hir_module(&db, file);
    let updated_fragment = first_general_expr_fragment(updated_hir.module());
    let third = runtime_fragment_backend_unit(&db, file, &updated_fragment)
        .expect("updated runtime fragment lowering should still succeed");

    assert!(!Arc::ptr_eq(&first, &third));
    assert_ne!(first.fingerprint(), third.fingerprint());
}
