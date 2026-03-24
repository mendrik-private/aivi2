use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use aivi_query::{
    RootDatabase, SourceFile, all_diagnostics, exported_names, format_file, hir_module,
    parsed_file, symbol_index,
};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
        .join(relative)
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "aivi-query-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be creatable");
        Self { path }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("temporary parent directories should be creatable");
        }
        fs::write(&path, text).expect("temporary file should be writable");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

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

#[test]
fn hir_queries_resolve_workspace_imports_and_respect_explicit_exports() {
    let root = fixture_path("milestone-2/valid/use-member-imports");
    let main_path = root.join("main.aivi");
    let main_text = fs::read_to_string(&main_path).expect("fixture should be readable");

    let db = RootDatabase::new();
    let main = SourceFile::new(&db, main_path.clone(), main_text);

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "workspace fixture should lower without HIR errors: {:?}",
        hir.hir_diagnostics()
    );

    let network_path = root.join("aivi/network.aivi");
    let network = db
        .file_at_path(&network_path)
        .expect("imported workspace module should be loaded lazily");
    let exported = exported_names(&db, network);
    assert!(exported.find("http").is_some());
    assert!(exported.find("Request").is_some());
    assert!(
        exported.find("mailbox").is_none(),
        "explicit exports should hide unexported top-level values"
    );
}

#[test]
fn changing_an_imported_file_invalidates_transitive_hir_dependents() {
    let workspace = TempDir::new("workspace-invalidation");
    let main_path = workspace.write(
        "main.aivi",
        "use shared.types (\n    Greeting\n)\n\ntype Welcome = Greeting\n",
    );
    let shared_path = workspace.write(
        "shared/types.aivi",
        "type Greeting = Text\n\nexport Greeting\n",
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );
    let shared = SourceFile::new(
        &db,
        shared_path.clone(),
        fs::read_to_string(&shared_path).expect("shared fixture should exist"),
    );

    let first = hir_module(&db, main);
    assert!(
        first.hir_diagnostics().is_empty(),
        "initial workspace should lower cleanly: {:?}",
        first.hir_diagnostics()
    );

    assert!(shared.set_text(
        &db,
        "type Salutation = Text\n\nexport Salutation\n".to_owned()
    ));

    let second = hir_module(&db, main);
    assert!(
        !Arc::ptr_eq(&first, &second),
        "changing an imported file should invalidate dependent HIR"
    );
    assert!(
        second
            .hir_diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code.as_ref())
            .any(|code| code.to_string() == "hir::unknown-imported-name"),
        "dependents should report a fresh unknown-imported-name diagnostic after the import disappears: {:?}",
        second.hir_diagnostics()
    );
}
