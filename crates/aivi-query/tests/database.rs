use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use aivi_query::{
    RootDatabase, SourceFile, all_diagnostics, exported_names, format_file, hir_module,
    parsed_file, symbol_index,
};

#[test]
fn open_file_reuses_existing_handle_for_the_same_path() {
    let db = RootDatabase::new();

    let first = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "val answer = 42".to_owned(),
    );
    let reopened = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "val answer = 42".to_owned(),
    );

    assert_eq!(first, reopened);
    assert_eq!(db.file_at_path(Path::new("main.aivi")), Some(first));
    assert_eq!(db.files().len(), 1);
}

#[test]
fn parsed_and_hir_queries_reuse_cached_snapshots_until_text_changes() {
    let db = RootDatabase::new();
    let file = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "val answer = 42".to_owned(),
    );

    let parsed_first = parsed_file(&db, file);
    let parsed_second = parsed_file(&db, file);
    assert!(Arc::ptr_eq(&parsed_first, &parsed_second));
    assert!(Arc::ptr_eq(&file.source(&db), &parsed_first.source_arc()));

    let hir_first = hir_module(&db, file);
    let hir_second = hir_module(&db, file);
    assert!(Arc::ptr_eq(&hir_first, &hir_second));
    assert!(Arc::ptr_eq(
        &hir_first.symbols_arc(),
        &symbol_index(&db, file)
    ));
    assert_eq!(hir_first.symbols()[0].name, "answer");
    assert_eq!(exported_names(&db, file).0[0].name, "answer");

    assert!(file.set_text(&db, "val total = 7".to_owned()));

    let parsed_third = parsed_file(&db, file);
    let hir_third = hir_module(&db, file);
    assert!(!Arc::ptr_eq(&parsed_first, &parsed_third));
    assert!(!Arc::ptr_eq(&hir_first, &hir_third));
    assert_eq!(hir_third.symbols()[0].name, "total");
}

#[test]
fn formatting_and_diagnostics_follow_the_current_file_revision() {
    let db = RootDatabase::new();
    let file = SourceFile::new(&db, PathBuf::from("main.aivi"), "val answer=42".to_owned());

    let original = file.text(&db);
    let formatted = format_file(&db, file).expect("known files should format");
    assert_ne!(formatted, original);
    assert!(!file.set_text(&db, original));

    assert!(file.set_text(&db, "val = 42".to_owned()));
    let diagnostics = all_diagnostics(&db, file);
    assert!(!diagnostics.is_empty());
}
