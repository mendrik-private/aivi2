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

fn stdlib_path(relative: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib")
        .join(relative);
    fs::canonicalize(&path).unwrap_or(path)
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
        "value answer = 42".to_owned(),
    );
    let reopened = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "value answer = 42".to_owned(),
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
        "value answer = 42".to_owned(),
    );

    let parsed_first = parsed_file(&db, file);
    let parsed_second = parsed_file(&db, file);
    assert!(Arc::ptr_eq(&parsed_first, &parsed_second));
    let current_source = file.source(&db);
    assert_eq!(current_source.path(), parsed_first.source().path());
    assert_eq!(current_source.text(), parsed_first.source().text());

    let hir_first = hir_module(&db, file);
    let hir_second = hir_module(&db, file);
    assert!(Arc::ptr_eq(&hir_first, &hir_second));
    assert!(Arc::ptr_eq(
        &hir_first.symbols_arc(),
        &symbol_index(&db, file)
    ));
    assert_eq!(hir_first.symbols()[0].name, "answer");
    assert_eq!(exported_names(&db, file).0[0].name, "answer");

    assert!(file.set_text(&db, "value total = 7".to_owned()));

    let parsed_third = parsed_file(&db, file);
    let hir_third = hir_module(&db, file);
    assert!(!Arc::ptr_eq(&parsed_first, &parsed_third));
    assert!(!Arc::ptr_eq(&hir_first, &hir_third));
    assert_eq!(hir_third.symbols()[0].name, "total");
}

#[test]
fn formatting_and_diagnostics_follow_the_current_file_revision() {
    let db = RootDatabase::new();
    let file = SourceFile::new(
        &db,
        PathBuf::from("main.aivi"),
        "value answer=42".to_owned(),
    );

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
fn hir_queries_fallback_to_bundled_stdlib_modules() {
    let workspace = TempDir::new("bundled-stdlib-fallback");
    let main_path = workspace.write(
        "main.aivi",
        "use aivi.bundledsmoketest (\n    bundledSentinel\n    BundledToken\n)\n\ntype Alias = BundledToken\nvalue marker = bundledSentinel\n",
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "bundled stdlib import fallback should lower cleanly: {:?}",
        hir.hir_diagnostics()
    );

    let bundled_module = db
        .file_at_path(&stdlib_path("aivi/bundledsmoketest.aivi"))
        .expect("bundled stdlib module should be loaded lazily");
    let bundled_support = db
        .file_at_path(&stdlib_path("aivi/bundledsmokesupport.aivi"))
        .expect("bundled stdlib dependencies should also resolve lazily");

    let bundled_exports = exported_names(&db, bundled_module);
    assert!(bundled_exports.find("bundledSentinel").is_some());
    assert!(bundled_exports.find("BundledToken").is_some());
    assert!(
        exported_names(&db, bundled_support)
            .find("bundledSupportSentinel")
            .is_some()
    );
}

#[test]
fn hir_queries_reject_non_exported_legacy_stdlib_names() {
    let workspace = TempDir::new("bundled-stdlib-export-precedence");
    let main_path = workspace.write(
        "main.aivi",
        "use aivi.fs (\n    FsError\n    readText\n)\n\ntype Alias = FsError\n",
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code.as_ref())
            .any(|code| code.to_string() == "hir::unknown-imported-name"),
        "legacy standalone names that are not exported by the bundled stdlib should stay unknown: {:?}",
        hir.hir_diagnostics()
    );

    let fs_module = db
        .file_at_path(&stdlib_path("aivi/fs.aivi"))
        .expect("bundled stdlib module should be loaded lazily");
    let exported = exported_names(&db, fs_module);
    assert!(exported.find("FsError").is_some());
    assert!(exported.find("readText").is_none());
}

#[test]
fn hir_queries_matrix_module_exports_public_api() {
    let workspace = TempDir::new("bundled-stdlib-matrix-api");
    let main_path = workspace.write(
        "main.aivi",
        concat!(
            "use aivi.matrix (\n",
            "    Matrix\n",
            "    MatrixError\n",
            "    init\n",
            "    width\n",
            "    replaceAt\n",
            ")\n",
            "\n",
            "type Int -> Int -> Int\n",
            "func cell = x y =>\n",
            "    x + y\n",
            "\n",
            "value board : Result MatrixError (Matrix Int) = init 2 2 cell\n",
        ),
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "matrix stdlib import surface should lower cleanly: {:?}",
        hir.hir_diagnostics()
    );

    let matrix_module = db
        .file_at_path(&stdlib_path("aivi/matrix.aivi"))
        .expect("bundled matrix module should be loaded lazily");
    let exported = exported_names(&db, matrix_module);
    for name in [
        "Matrix",
        "MatrixError",
        "NegativeWidth",
        "NegativeHeight",
        "RaggedRows",
        "init",
        "fromRows",
        "width",
        "height",
        "rows",
        "row",
        "at",
        "replaceAt",
    ] {
        assert!(
            exported.find(name).is_some(),
            "expected matrix module to export `{name}`"
        );
    }
    assert!(
        exported.find("MkMatrix").is_none(),
        "opaque matrix constructor should stay hidden"
    );
}

#[test]
fn hir_queries_fallback_to_bundled_root_and_prelude_modules() {
    let workspace = TempDir::new("bundled-root-prelude-fallback");
    let main_path = workspace.write(
        "main.aivi",
        "use aivi (\n    Option\n    Result\n    Validation\n    Signal\n    Task\n    Some\n    None\n    Ok\n    Err\n    Valid\n    Invalid\n)\n\nuse aivi.prelude (\n    Int\n    Bool\n    Text\n    List\n    Eq\n    Default\n    Functor\n    Applicative\n    Monad\n    Foldable\n    getOrElse\n    withDefault\n    length\n    head\n    join\n)\n\ntype NameSignal = Signal Text\ntype CountTask = Task Text Int\ntype CheckedName = Validation Text Text\n\nvalue maybeName:Option Text = Some \"Ada\"\nvalue missingName:Option Text = None\nvalue chosenName:Text = getOrElse \"guest\" missingName\n\nvalue okCount:Result Text Int = Ok 2\nvalue errCount:Result Text Int = Err \"missing\"\nvalue chosenCount:Int = withDefault 0 okCount\n\nvalue checkedName:CheckedName = Valid \"Ada\"\nvalue nameCount:Int = length [\"Ada\", \"Grace\"]\nvalue firstName:Option Text = head [\"Ada\", \"Grace\"]\nvalue labels:Text = join \", \" [\"Ada\", \"Grace\"]\nvalue sameCount:Bool = chosenCount == 2\n",
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "bundled root/prelude imports should lower cleanly: {:?}",
        hir.hir_diagnostics()
    );

    let root_module = db
        .file_at_path(&stdlib_path("aivi.aivi"))
        .expect("bundled root stdlib module should be loaded lazily");
    let prelude_module = db
        .file_at_path(&stdlib_path("aivi/prelude.aivi"))
        .expect("bundled prelude stdlib module should be loaded lazily");

    let root_exports = exported_names(&db, root_module);
    assert!(root_exports.find("Option").is_some());
    assert!(root_exports.find("Some").is_some());
    assert!(root_exports.find("Eq").is_some());

    let prelude_exports = exported_names(&db, prelude_module);
    assert!(prelude_exports.find("Int").is_some());
    assert!(prelude_exports.find("getOrElse").is_some());
    assert!(prelude_exports.find("length").is_some());
    assert!(prelude_exports.find("join").is_some());
}

#[test]
fn hir_queries_fallback_to_bundled_phase_two_boundary_modules() {
    let workspace = TempDir::new("bundled-phase-two-boundaries");
    workspace.write("aivi.toml", "");
    let main_path = workspace.write(
        "main.aivi",
        r#"use aivi.duration (
    Duration
)

use aivi.http (
    HttpError
    Timeout
    DecodeFailure
    RequestFailure
    HttpHeaders
    HttpQuery
    HttpResponse
    DecodeMode
    Strict
    Retry
    HttpSource
)

use aivi.timer (
    TimerTick
    TimerReady
)

use aivi.log (
    LogLevel
    Debug
    Error
    LogContext
    LogEntry
    LogError
    LogSink
)

type User = {
    id: Int,
    name: Text
}

value headers:HttpHeaders =
    Map {
        "Authorization": "Bearer demo"
    }

value query:HttpQuery =
    Map {
        "page": "1"
    }

value decodeMode:DecodeMode =
    Strict

type RetryBudget = Retry

type UsersResponse = (HttpResponse (List User))
type UsersTask = (Task Text (List User))

@source http "https://api.example.com"
signal api : HttpSource

signal users : Signal UsersResponse = api.get "/users"

@source timer.every 120 with {
    immediate: True,
    coalesce: True
}
signal tick : Signal TimerTick

@source timer.after 1000
signal ready : Signal TimerReady

value timeoutError:HttpError =
    Timeout

value decodeError:HttpError =
    DecodeFailure "bad-json"

value requestError:HttpError =
    RequestFailure "offline"

value level:LogLevel =
    Debug

value context:LogContext =
    Map {
        "module": "query"
    }

value entry:LogEntry = {
    level: level,
    message: "loaded",
    context: context
}

type Writer = LogSink
type CurrentLogTask = (Task LogError Unit)
type CurrentLogError = LogError

type PollDelay = Duration

value errorLevel:LogLevel =
    Error
"#,
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "bundled phase-two boundary imports should lower cleanly: {:?}",
        hir.hir_diagnostics()
    );

    let http_module = db
        .file_at_path(&stdlib_path("aivi/http.aivi"))
        .expect("bundled http stdlib module should be loaded lazily");
    let timer_module = db
        .file_at_path(&stdlib_path("aivi/timer.aivi"))
        .expect("bundled timer stdlib module should be loaded lazily");
    let log_module = db
        .file_at_path(&stdlib_path("aivi/log.aivi"))
        .expect("bundled log stdlib module should be loaded lazily");
    let duration_module = db
        .file_at_path(&stdlib_path("aivi/duration.aivi"))
        .expect("bundled duration stdlib dependency should be loaded lazily");

    let http_exports = exported_names(&db, http_module);
    assert!(http_exports.find("HttpError").is_some());
    assert!(http_exports.find("RequestFailure").is_some());
    assert!(http_exports.find("HttpResponse").is_some());
    assert!(http_exports.find("Retry").is_some());

    let timer_exports = exported_names(&db, timer_module);
    assert!(timer_exports.find("TimerTick").is_some());
    assert!(timer_exports.find("TimerReady").is_some());

    let log_exports = exported_names(&db, log_module);
    assert!(log_exports.find("LogLevel").is_some());
    assert!(log_exports.find("Debug").is_some());
    assert!(log_exports.find("LogSink").is_some());

    assert!(
        exported_names(&db, duration_module)
            .find("Duration")
            .is_some()
    );
}

#[test]
fn hir_queries_prefer_workspace_modules_over_bundled_stdlib_fallback() {
    let workspace = TempDir::new("bundled-stdlib-overlay");
    workspace.write("aivi.toml", "");
    let main_path = workspace.write(
        "main.aivi",
        "use aivi.bundledsmoketest (\n    workspaceOnly\n    LocalToken\n)\n\ntype Alias = LocalToken\nvalue marker = workspaceOnly\n",
    );
    let local_module_path = workspace.write(
        "aivi/bundledsmoketest.aivi",
        "use aivi.bundledsmokesupport (\n    bundledSupportSentinel\n)\n\nvalue workspaceOnly:Text = bundledSupportSentinel\ntype LocalToken = Text\n\nexport (workspaceOnly, LocalToken)\n",
    );

    let db = RootDatabase::new();
    let main = SourceFile::new(
        &db,
        main_path.clone(),
        fs::read_to_string(&main_path).expect("main fixture should exist"),
    );

    let hir = hir_module(&db, main);
    assert!(
        hir.hir_diagnostics().is_empty(),
        "workspace module should override bundled stdlib fallback: {:?}",
        hir.hir_diagnostics()
    );

    let local_module = db
        .file_at_path(&local_module_path)
        .expect("workspace override should satisfy the import");
    assert!(
        exported_names(&db, local_module)
            .find("workspaceOnly")
            .is_some()
    );
    assert!(
        db.file_at_path(&stdlib_path("aivi/bundledsmoketest.aivi"))
            .is_none(),
        "bundled stdlib should not load when a workspace module already exists"
    );
    assert!(
        db.file_at_path(&stdlib_path("aivi/bundledsmokesupport.aivi"))
            .is_some(),
        "workspace overrides should still be able to fall back to bundled stdlib dependencies"
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
