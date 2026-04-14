use aivi_base::SourceDatabase;
use aivi_hir::{
    Item, SourceLifecycleNodeOutcome, SourceProviderRef, ValidationMode,
    elaborate_source_lifecycles, lower_module, validate_module,
};
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
    let lowered = lower_module(&parsed.module);
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
fn db_connect_lifecycle_keeps_reactive_config_and_active_when() {
    let lowered = lower_text("db-connect-lifecycle.aivi", DB_SOURCE_FIXTURE);
    let validation = validate_module(lowered.module(), ValidationMode::RequireResolvedNames);
    assert!(
        validation.is_ok(),
        "db source fixture should validate cleanly: {:?}",
        validation.diagnostics()
    );

    let lifecycle = elaborate_source_lifecycles(lowered.module());
    let db = lifecycle
        .nodes()
        .iter()
        .find(|node| node.owner == item_id(lowered.module(), "db"))
        .expect("db source lifecycle should exist");

    match &db.outcome {
        SourceLifecycleNodeOutcome::Planned(plan) => {
            assert_eq!(
                plan.provider,
                SourceProviderRef::Builtin(BuiltinSourceProvider::DbConnect)
            );
            assert_eq!(plan.arguments.len(), 1);
            assert_eq!(
                plan.reconfiguration_dependencies,
                vec![item_id(lowered.module(), "config")]
            );
            assert!(
                plan.explicit_triggers.is_empty(),
                "db.connect should not register explicit trigger signals"
            );
            assert_eq!(
                plan.options
                    .iter()
                    .map(|option| option.option_name.text().to_owned())
                    .collect::<Vec<_>>(),
                vec!["pool".to_owned(), "activeWhen".to_owned()]
            );
            let active_when = plan
                .active_when
                .as_ref()
                .expect("db.connect should preserve activeWhen");
            assert_eq!(active_when.option_name.text(), "activeWhen");
            assert_eq!(
                active_when.signal,
                Some(item_id(lowered.module(), "enabled"))
            );
        }
        other => panic!("expected planned db.connect lifecycle, found {other:?}"),
    }
}

#[test]
fn db_live_lifecycle_keeps_changed_refresh_and_active_when() {
    let lowered = lower_text("db-live-lifecycle.aivi", DB_SOURCE_FIXTURE);
    let validation = validate_module(lowered.module(), ValidationMode::RequireResolvedNames);
    assert!(
        validation.is_ok(),
        "db source fixture should validate cleanly: {:?}",
        validation.diagnostics()
    );

    let lifecycle = elaborate_source_lifecycles(lowered.module());
    let rows = lifecycle
        .nodes()
        .iter()
        .find(|node| node.owner == item_id(lowered.module(), "rows"))
        .expect("rows source lifecycle should exist");

    match &rows.outcome {
        SourceLifecycleNodeOutcome::Planned(plan) => {
            assert_eq!(
                plan.provider,
                SourceProviderRef::Builtin(BuiltinSourceProvider::DbLive)
            );
            assert_eq!(plan.arguments.len(), 1);
            assert!(
                plan.reconfiguration_dependencies.is_empty(),
                "db.live should keep refreshOn and activeWhen outside the reactive argument list"
            );
            assert_eq!(plan.explicit_triggers.len(), 1);
            assert_eq!(plan.explicit_triggers[0].option_name.text(), "refreshOn");
            assert_eq!(
                plan.explicit_triggers[0].signal,
                Some(item_id(lowered.module(), "rows#trigger"))
            );
            let active_when = plan
                .active_when
                .as_ref()
                .expect("db.live should preserve activeWhen");
            assert_eq!(active_when.option_name.text(), "activeWhen");
            assert_eq!(
                active_when.signal,
                Some(item_id(lowered.module(), "enabled"))
            );
        }
        other => panic!("expected planned db.live lifecycle, found {other:?}"),
    }
}
