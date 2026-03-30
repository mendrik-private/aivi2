use aivi_base::SourceDatabase;
use aivi_hir::{Item, lower_module as lower_hir_module};
use aivi_runtime::{RuntimeSourceProvider, assemble_hir_runtime};
use aivi_syntax::parse_module;
use aivi_typing::BuiltinSourceProvider;

const DB_SOURCE_FIXTURE: &str = r#"
type DbError =
  | ConnectionFailed Text
  | QueryFailed Text

type Connection = {
    database: Text
}

type TableRef A = {
    changed: Signal Unit
}

signal config : Signal Connection = {
    database: "fixtures/live.sqlite"
}

signal enabled = True
signal usersChanged : Signal Unit

value users : TableRef Int = {
    changed: usersChanged
}

value loadUsers : Task DbError (List Int) =
    pure [1, 2, 3]

@source db.connect config with {
    pool: 4,
    activeWhen: enabled
}
signal db : Signal (Result DbError Connection)

@source db.live loadUsers with {
    refreshOn: users.changed,
    activeWhen: enabled
}
signal rows : Signal (Result DbError (List Int))
"#;

fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let lowered = lower_hir_module(&parsed.module);
    assert!(
        !lowered.has_errors(),
        "fixture {path} should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    lowered
}

fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(item) if item.name.text() == name => Some(item_id),
            Item::Function(item) if item.name.text() == name => Some(item_id),
            Item::Signal(item) if item.name.text() == name => Some(item_id),
            Item::Type(item) if item.name.text() == name => Some(item_id),
            Item::Class(item) if item.name.text() == name => Some(item_id),
            Item::Domain(item) if item.name.text() == name => Some(item_id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected item named {name}"))
}

#[test]
fn assembles_db_connect_spec_with_reactive_config_and_active_when() {
    let lowered = lower_text("runtime-db-connect-spec.aivi", DB_SOURCE_FIXTURE);
    let assembly =
        assemble_hir_runtime(lowered.module()).expect("db source fixture should assemble");

    let config = assembly
        .signal(item_id(lowered.module(), "config"))
        .expect("config signal binding should exist");
    let enabled = assembly
        .signal(item_id(lowered.module(), "enabled"))
        .expect("enabled signal binding should exist");
    let db = assembly
        .source_by_owner(item_id(lowered.module(), "db"))
        .expect("db source binding should exist");

    assert_eq!(
        db.spec.provider,
        RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbConnect)
    );
    assert_eq!(
        db.spec.reconfiguration_dependencies.as_ref(),
        &[config.signal()]
    );
    assert!(
        db.spec.explicit_triggers.is_empty(),
        "db.connect should not assemble any explicit trigger signals"
    );
    assert_eq!(db.spec.active_when, Some(enabled.signal()));
}

#[test]
fn assembles_db_live_spec_with_changed_refresh_and_active_when() {
    let lowered = lower_text("runtime-db-live-spec.aivi", DB_SOURCE_FIXTURE);
    let assembly =
        assemble_hir_runtime(lowered.module()).expect("db source fixture should assemble");

    let changed = assembly
        .signal(item_id(lowered.module(), "usersChanged"))
        .expect("usersChanged signal binding should exist");
    let enabled = assembly
        .signal(item_id(lowered.module(), "enabled"))
        .expect("enabled signal binding should exist");
    let rows = assembly
        .source_by_owner(item_id(lowered.module(), "rows"))
        .expect("rows source binding should exist");

    assert_eq!(
        rows.spec.provider,
        RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbLive)
    );
    assert!(
        rows.spec.reconfiguration_dependencies.is_empty(),
        "db.live should not treat refreshOn or activeWhen as ordinary reactive inputs"
    );
    assert_eq!(rows.spec.explicit_triggers.as_ref(), &[changed.signal()]);
    assert_eq!(rows.spec.active_when, Some(enabled.signal()));
}
