use std::{fs, path::PathBuf};

use aivi_base::{Severity, SourceDatabase};
use aivi_syntax::parse_module;
use aivi_typing::{BuiltinSourceProvider, Kind};

use super::{lower_module, path_text};
use crate::{
    ApplicativeSpineHead, BuiltinTerm, BuiltinType, ClusterFinalizer, ClusterPresentation,
    DecoratorPayload, DomainMemberKind, DomainMemberResolution, ExportResolution, ExprKind,
    HoistKindFilter, ImportBindingMetadata, ImportBundleKind, ImportValueType, IntrinsicValue,
    Item, LiteralSuffixResolution, PipeStageKind, ReactiveUpdateBodyMode, RecordRowTransform,
    RecurrenceWakeupDecoratorKind, ResolutionState, SourceProviderRef, TermResolution, TextSegment,
    TypeItemBody, TypeKind, TypeResolution, ValidationMode, exports,
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

fn lower_text(path: &str, text: &str) -> super::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    lower_module(&parsed.module)
}

fn lower_fixture(path: &str) -> super::LoweringResult {
    let text = fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
    lower_text(path, &text)
}

#[test]
fn did_you_mean_suggestion_for_misspelled_binding() {
    // "couner" is 2 edits from "counter" — should trigger a "did you mean" hint.
    let result = lower_text(
        "did-you-mean.aivi",
        "value counter = 42\nvalue result = couner\n",
    );
    assert!(result.has_errors(), "expected an error for unknown term");
    let has_help = result
        .diagnostics()
        .iter()
        .any(|d| d.help.iter().any(|h| h.contains("counter")));
    assert!(
        has_help,
        "expected a 'did you mean `counter`?' hint, got diagnostics: {:?}",
        result.diagnostics()
    );
}

#[test]
fn no_did_you_mean_for_very_different_binding() {
    // "xyz" has no close match to "counter" — no suggestion should appear.
    let result = lower_text(
        "no-suggestion.aivi",
        "value counter = 42\nvalue result = xyz\n",
    );
    assert!(result.has_errors(), "expected an error for unknown term");
    let has_help = result.diagnostics().iter().any(|d| !d.help.is_empty());
    assert!(
        !has_help,
        "expected no 'did you mean' hint for very different name, got: {:?}",
        result.diagnostics()
    );
}

fn find_ambient_named_item<'a>(module: &'a crate::Module, name: &str) -> &'a Item {
    module
        .ambient_items()
        .iter()
        .map(|item_id| &module.items()[*item_id])
        .find(|item| match item {
            Item::Type(item) => item.name.text() == name,
            Item::Value(item) => item.name.text() == name,
            Item::Function(item) => item.name.text() == name,
            Item::Signal(item) => item.name.text() == name,
            Item::Class(item) => item.name.text() == name,
            Item::Domain(item) => item.name.text() == name,
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => false,
        })
        .unwrap_or_else(|| panic!("expected to find ambient item `{name}`"))
}

fn find_named_item<'a>(module: &'a crate::Module, name: &str) -> &'a Item {
    module
        .root_items()
        .iter()
        .map(|item_id| &module.items()[*item_id])
        .find(|item| match item {
            Item::Type(item) => item.name.text() == name,
            Item::Value(item) => item.name.text() == name,
            Item::Function(item) => item.name.text() == name,
            Item::Signal(item) => item.name.text() == name,
            Item::Class(item) => item.name.text() == name,
            Item::Domain(item) => item.name.text() == name,
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => false,
        })
        .unwrap_or_else(|| panic!("expected to find named item `{name}`"))
}

fn find_signal<'a>(module: &'a crate::Module, name: &str) -> &'a crate::SignalItem {
    match find_named_item(module, name) {
        Item::Signal(item) => item,
        other => panic!("expected `{name}` to be a signal item, found {other:?}"),
    }
}

fn find_value<'a>(module: &'a crate::Module, name: &str) -> &'a crate::ValueItem {
    match find_named_item(module, name) {
        Item::Value(item) => item,
        other => panic!("expected `{name}` to be a value item, found {other:?}"),
    }
}

fn resolved_signal_name(module: &crate::Module, expr_id: crate::ExprId) -> String {
    let ExprKind::Name(reference) = &module.exprs()[expr_id].kind else {
        panic!("expected expression to lower as a direct name reference");
    };
    let ResolutionState::Resolved(TermResolution::Item(item_id)) = reference.resolution else {
        panic!("expected expression to resolve to a same-module item");
    };
    match &module.items()[item_id] {
        Item::Signal(signal) => signal.name.text().to_owned(),
        other => panic!("expected resolved item to be a signal, found {other:?}"),
    }
}

fn signal_dependency_names(module: &crate::Module, item: &crate::SignalItem) -> Vec<String> {
    item.signal_dependencies
        .iter()
        .map(|item_id| match &module.items()[*item_id] {
            Item::Signal(signal) => signal.name.text().to_owned(),
            other => {
                panic!("expected signal dependency to point at a signal item, found {other:?}")
            }
        })
        .collect()
}

fn signal_item_names(module: &crate::Module, dependencies: &[crate::ItemId]) -> Vec<String> {
    dependencies
        .iter()
        .map(|item_id| match &module.items()[*item_id] {
            Item::Signal(signal) => signal.name.text().to_owned(),
            other => {
                panic!("expected source dependency to point at a signal item, found {other:?}")
            }
        })
        .collect()
}

#[test]
fn lowers_request_resource_companions_and_rewrites_resource_members() {
    let lowered = lower_text(
        "resource-companions.aivi",
        r#"signal refresh : Signal Unit
signal enabled = True

@source http.get "/users" with {
  refreshOn: refresh,
  activeWhen: enabled
}
signal usersResult : Signal (Result Text (List Text))

value rerun = usersResult.run
value success = usersResult.success
value failure = usersResult.error
value loading = usersResult.loading
"#,
    );
    assert!(
        !lowered.has_errors(),
        "resource companions should lower cleanly, got diagnostics: {:?}",
        lowered.diagnostics()
    );

    let module = lowered.module();
    let users_result = find_signal(module, "usersResult");
    let run = find_signal(module, "usersResult#run");
    let trigger = find_signal(module, "usersResult#trigger");
    let success = find_signal(module, "usersResult#success");
    let failure = find_signal(module, "usersResult#error");
    let loading = find_signal(module, "usersResult#loading");

    assert!(
        run.body.is_none(),
        "resource run companion should lower as an input-backed signal"
    );
    assert_eq!(
        signal_dependency_names(module, trigger),
        vec!["refresh".to_owned(), "usersResult#run".to_owned()],
        "resource trigger helper should depend on the explicit trigger and the hidden run input"
    );
    assert_eq!(
        signal_dependency_names(module, success),
        vec!["usersResult".to_owned()],
        "success companion should depend on the raw source signal"
    );
    assert_eq!(
        signal_dependency_names(module, failure),
        vec!["usersResult".to_owned()],
        "error companion should depend on the raw source signal"
    );
    let loading_dependencies = signal_dependency_names(module, loading);
    assert!(
        loading_dependencies.contains(&"enabled".to_owned()),
        "loading companion should track activeWhen"
    );
    assert!(
        loading_dependencies.contains(&"usersResult#trigger".to_owned()),
        "loading companion should react to hidden retriggers"
    );
    assert!(
        loading_dependencies.contains(&"usersResult".to_owned()),
        "loading companion should settle on source publications"
    );

    let source_dependencies = signal_item_names(
        module,
        &users_result
            .source_metadata
            .as_ref()
            .unwrap()
            .lifecycle_dependencies
            .explicit_triggers,
    );
    assert_eq!(
        source_dependencies,
        vec!["usersResult#trigger".to_owned()],
        "request-like source should wake from the synthesized trigger helper"
    );

    assert_eq!(
        resolved_signal_name(module, find_value(module, "rerun").body),
        "usersResult#run"
    );
    assert_eq!(
        resolved_signal_name(module, find_value(module, "success").body),
        "usersResult#success"
    );
    assert_eq!(
        resolved_signal_name(module, find_value(module, "failure").body),
        "usersResult#error"
    );
    assert_eq!(
        resolved_signal_name(module, find_value(module, "loading").body),
        "usersResult#loading"
    );
}

#[test]
fn lowers_valid_fixture_corpus() {
    for path in [
        "milestone-2/valid/local-top-level-refs/main.aivi",
        "milestone-2/valid/use-member-imports/main.aivi",
        "milestone-2/valid/use-member-import-aliases/main.aivi",
        "milestone-2/valid/source-provider-contract-declarations/main.aivi",
        "milestone-2/valid/custom-source-provider-wakeup/main.aivi",
        "milestone-2/valid/custom-source-recurrence-wakeup/main.aivi",
        "milestone-2/valid/source-decorator-signals/main.aivi",
        "milestone-2/valid/source-option-contract-parameters/main.aivi",
        "milestone-2/valid/source-option-imported-binding-match/main.aivi",
        "milestone-2/valid/source-option-constructor-applications/main.aivi",
        "milestone-2/valid/applicative-clusters/main.aivi",
        "milestone-2/valid/markup-control-nodes/main.aivi",
        "milestone-2/valid/class-declarations/main.aivi",
        "milestone-2/valid/instance-declarations/main.aivi",
        "milestone-2/valid/domain-declarations/main.aivi",
        "milestone-2/valid/domain-member-resolution/main.aivi",
        "milestone-2/valid/domain-literal-suffixes/main.aivi",
        "milestone-2/valid/type-kinds/main.aivi",
        "milestone-2/valid/pipe-branch-and-join/main.aivi",
        "milestone-2/valid/pipe-fanout-carriers/main.aivi",
        "milestone-2/valid/result-block/main.aivi",
        "milestone-2/valid/pipe-accumulate-signal-wakeup/main.aivi",
        "milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi",
        "milestone-1/valid/records/record_shorthand_and_elision.aivi",
        "milestone-1/valid/sources/source_declarations.aivi",
        "milestone-1/valid/strings/text_and_regex.aivi",
        "milestone-1/valid/top-level/declarations.aivi",
        "milestone-1/valid/pipes/pipe_algebra.aivi",
        "milestone-1/valid/pipes/applicative_clusters.aivi",
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "expected {path} to validate as resolved HIR, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn lowers_record_row_transform_aliases_into_explicit_hir_types() {
    let lowered = lower_text(
        "record-row-transforms.aivi",
        concat!(
            "type User = { id: Int, name: Text, nickname: Option Text, createdAt: Text }\n",
            "type Public = Pick (id, name) User\n",
            "type Patch = User |> Omit (createdAt) |> Optional (name, nickname)\n",
            "type Snake = Rename { createdAt: created_at } User\n",
        ),
    );
    assert!(
        !lowered.has_errors(),
        "record row transform aliases should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let module = lowered.module();

    let Item::Type(public) = find_named_item(module, "Public") else {
        panic!("expected `Public` to be a type item");
    };
    let TypeItemBody::Alias(public_alias) = public.body else {
        panic!("expected `Public` to be an alias");
    };
    match &module.types()[public_alias].kind {
        TypeKind::RecordTransform { transform, source } => {
            assert!(matches!(transform, RecordRowTransform::Pick(labels) if labels.len() == 2));
            assert!(matches!(
                &module.types()[*source].kind,
                TypeKind::Name(reference)
                    if reference.path.to_string() == "User"
            ));
        }
        other => panic!("expected `Public` to lower to a record transform, found {other:?}"),
    }

    let Item::Type(patch) = find_named_item(module, "Patch") else {
        panic!("expected `Patch` to be a type item");
    };
    let TypeItemBody::Alias(patch_alias) = patch.body else {
        panic!("expected `Patch` to be an alias");
    };
    match &module.types()[patch_alias].kind {
        TypeKind::RecordTransform { transform, source } => {
            assert!(matches!(
                transform,
                RecordRowTransform::Optional(labels) if labels.len() == 2
            ));
            assert!(matches!(
                &module.types()[*source].kind,
                TypeKind::RecordTransform {
                    transform: RecordRowTransform::Omit(labels),
                    ..
                } if labels.len() == 1
            ));
        }
        other => {
            panic!("expected `Patch` to lower to a nested record transform, found {other:?}")
        }
    }

    let Item::Type(snake) = find_named_item(module, "Snake") else {
        panic!("expected `Snake` to be a type item");
    };
    let TypeItemBody::Alias(snake_alias) = snake.body else {
        panic!("expected `Snake` to be an alias");
    };
    match &module.types()[snake_alias].kind {
        TypeKind::RecordTransform {
            transform: RecordRowTransform::Rename(renames),
            ..
        } => {
            assert_eq!(renames.len(), 1);
            assert_eq!(renames[0].from.text(), "createdAt");
            assert_eq!(renames[0].to.text(), "created_at");
        }
        other => panic!("expected `Snake` to lower to a rename transform, found {other:?}"),
    }
}

#[test]
fn lowering_reports_malformed_record_row_transform_shapes() {
    let lowered = lower_text(
        "invalid-record-row-transform.aivi",
        concat!(
            "type User = { id: Int, name: Text }\n",
            "type BrokenPick = Pick id User\n",
            "type BrokenRename = Rename (id) User\n",
        ),
    );
    let codes = lowered
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(
        codes.contains(&super::code("invalid-record-row-transform")),
        "expected malformed transforms to report invalid-record-row-transform, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn lowers_single_source_signal_merge_arms() {
    let lowered = lower_text(
        "single_source_merge.aivi",
        r#"signal ready : Signal Bool
signal left : Signal Int
signal right : Signal Int

signal total : Signal Int = ready
  ||> True => left + right
  ||> _ => 0
"#,
    );
    assert!(
        !lowered.has_errors(),
        "expected signal merge to lower cleanly, got diagnostics: {:?}",
        lowered.diagnostics()
    );

    let module = lowered.module();
    let total = find_signal(module, "total");
    // Default arm becomes the seed body, so only 1 reactive update.
    assert_eq!(total.reactive_updates.len(), 1);
    assert!(
        total.body.is_some(),
        "default arm should become the signal seed body"
    );
    assert_eq!(
        total.reactive_updates[0].body_mode,
        ReactiveUpdateBodyMode::OptionalPayload
    );
}

#[test]
fn lowers_merge_rejects_unknown_source_signal() {
    let lowered = lower_text(
        "merge_unknown_source.aivi",
        r#"signal total : Signal Int = nonexistent
  ||> True => 42
  ||> _ => 0
"#,
    );
    assert!(
        lowered.has_errors(),
        "expected unknown merge source to fail lowering"
    );
    assert!(lowered.diagnostics().iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            && diagnostic
                .message
                .contains("must name a previously declared signal")
    }));
}

#[test]
fn lowers_multi_source_signal_merge_arms() {
    let lowered = lower_text(
        "multi_source_merge.aivi",
        r#"type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down

signal heading : Signal Direction = event
  ||> Turn dir => dir
  ||> _ => Up

signal tickSeen : Signal Bool = event
  ||> Tick => True
  ||> _ => False
"#,
    );
    assert!(
        !lowered.has_errors(),
        "expected multi-source signal merge to lower cleanly, got diagnostics: {:?}",
        lowered.diagnostics()
    );

    let module = lowered.module();
    let heading = find_signal(module, "heading");
    let tick_seen = find_signal(module, "tickSeen");
    // Default arm becomes seed, so only 1 reactive update each.
    assert_eq!(heading.reactive_updates.len(), 1);
    assert_eq!(tick_seen.reactive_updates.len(), 1);
    assert!(
        heading.body.is_some(),
        "default arm should become heading's seed"
    );
    assert!(
        tick_seen.body.is_some(),
        "default arm should become tickSeen's seed"
    );
    assert_eq!(
        heading.reactive_updates[0].body_mode,
        ReactiveUpdateBodyMode::OptionalPayload
    );
}

#[test]
fn lowers_source_pattern_signal_merge_arms() {
    let lowered = lower_text(
        "source_pattern_merge.aivi",
        r#"type Key = Key Text
type Event = Tick | Turn Text

signal keyDown : Signal Key

signal event : Signal Event = keyDown
  ||> (Key "ArrowUp") => Turn "up"
  ||> _ => Tick
"#,
    );
    assert!(
        !lowered.has_errors(),
        "expected source-pattern merge to lower cleanly, got diagnostics: {:?}",
        lowered.diagnostics()
    );

    let module = lowered.module();
    let event = find_signal(module, "event");
    // Default arm becomes seed, so only 1 reactive update.
    assert_eq!(event.reactive_updates.len(), 1);
    assert!(
        event.body.is_some(),
        "default arm should become event's seed"
    );
    assert_eq!(
        event.reactive_updates[0].body_mode,
        ReactiveUpdateBodyMode::OptionalPayload
    );
    assert!(matches!(
        module.exprs()[event.reactive_updates[0].guard].kind,
        ExprKind::Pipe(_)
    ));
}

#[test]
fn lower_injects_ambient_typeclass_prelude() {
    let lowered = lower_text("ambient-prelude.aivi", "value answer:Int = 42\n");
    assert!(
        !lowered.has_errors(),
        "ambient prelude should lower cleanly, got diagnostics: {:?}",
        lowered.diagnostics()
    );
    let module = lowered.module();
    assert!(
        module.ambient_items().len() >= 10,
        "expected ambient prelude items to be injected"
    );
    assert!(
        matches!(find_ambient_named_item(module, "Ordering"), Item::Type(_)),
        "expected ambient Ordering type to be present"
    );
    assert!(
        matches!(find_ambient_named_item(module, "Default"), Item::Class(_)),
        "expected ambient Default class to be present"
    );
    let Item::Class(traversable) = find_ambient_named_item(module, "Traversable") else {
        panic!("expected ambient Traversable class");
    };
    let traverse = traversable
        .members
        .iter()
        .find(|member| member.name.text() == "traverse")
        .expect("Traversable should expose traverse");
    assert_eq!(
        traverse.context.len(),
        1,
        "expected traverse to keep its Applicative constraint"
    );
    let Item::Class(applicative) = find_ambient_named_item(module, "Applicative") else {
        panic!("expected ambient Applicative class");
    };
    assert!(
        !applicative.superclasses.is_empty(),
        "Applicative should retain its superclass edge"
    );
}

#[test]
fn ambient_prelude_prefers_builtin_names_over_user_shadowing() {
    let lowered = lower_text(
        "ambient-shadow-bool.aivi",
        r#"
type Bool = True | False

value answer:Int = 42
"#,
    );
    assert!(
        !lowered.has_errors(),
        "fixture should lower cleanly before validation: {:?}",
        lowered.diagnostics()
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "ambient prelude should validate even when the user shadows builtin Bool: {:?}",
        report.diagnostics()
    );

    let Item::Function(any_step) = find_ambient_named_item(lowered.module(), "__aivi_list_anyStep")
    else {
        panic!("expected `__aivi_list_anyStep` to lower as an ambient function");
    };
    let found_annotation = any_step.parameters[1]
        .annotation
        .expect("ambient helper parameter should retain its annotation");
    assert!(matches!(
        lowered.module().types()[found_annotation].kind,
        TypeKind::Name(ref reference)
            if matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Bool))
            )
    ));
}

#[test]
fn reports_invalid_fixture_corpus_but_keeps_structural_hir() {
    for path in [
        "milestone-2/invalid/duplicate-top-level-names/main.aivi",
        "milestone-2/invalid/duplicate-source-provider-contract/main.aivi",
        "milestone-2/invalid/unknown-imported-names/main.aivi",
        "milestone-2/invalid/unknown-decorator/main.aivi",
        "milestone-2/invalid/unresolved-names/main.aivi",
        "milestone-2/invalid/misplaced-control-branches/main.aivi",
        "milestone-2/invalid/source-decorator-non-signal/main.aivi",
        "milestone-2/invalid/unknown-import-module/main.aivi",
        "milestone-2/invalid/domain-recursive-carrier/main.aivi",
        "milestone-2/invalid/unpaired-truthy-falsy/main.aivi",
        "milestone-2/invalid/fanin-without-map/main.aivi",
        "milestone-2/invalid/cluster-ambient-projection/main.aivi",
        "milestone-2/invalid/orphan-recur-step/main.aivi",
        "milestone-2/invalid/unfinished-recurrence/main.aivi",
        "milestone-2/invalid/recurrence-continuation/main.aivi",
        "milestone-2/invalid/interpolated-pattern-text/main.aivi",
        "milestone-1/invalid/cluster_unfinished_gate.aivi",
        "milestone-1/invalid/source_unknown_option.aivi",
        "milestone-2/invalid/source-duplicate-option/main.aivi",
        "milestone-2/invalid/source-provider-without-variant/main.aivi",
        "milestone-2/invalid/source-legacy-quantity-option/main.aivi",
    ] {
        let lowered = lower_fixture(path);
        assert!(
            lowered.has_errors(),
            "expected {path} to fail HIR lowering, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "expected {path} to keep structurally valid HIR, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_kind_invalid_fixtures() {
    for (path, code_name) in [
        (
            "milestone-2/invalid/overapplied-type-constructor/main.aivi",
            "invalid-type-application",
        ),
        (
            "milestone-2/invalid/underapplied-domain-constructor/main.aivi",
            "expected-kind-mismatch",
        ),
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before kind validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
            "expected {path} to report {code_name}, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_recurrence_target_invalid_fixtures() {
    for (path, code_name) in [
        (
            "milestone-2/invalid/unknown-recurrence-target/main.aivi",
            "unknown-recurrence-target",
        ),
        (
            "milestone-2/invalid/unsupported-recurrence-target/main.aivi",
            "unsupported-recurrence-target",
        ),
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before recurrence target validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
            "expected {path} to report {code_name}, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_source_contract_invalid_fixtures() {
    for (path, code_name) in [
        (
            "milestone-2/invalid/source-contract-missing-type/main.aivi",
            "missing-source-contract-type",
        ),
        (
            "milestone-2/invalid/source-contract-arity-mismatch/main.aivi",
            "source-contract-type-arity",
        ),
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before source contract validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
            "expected {path} to report {code_name}, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_source_option_type_invalid_fixtures() {
    for path in [
        "milestone-2/invalid/source-option-type-mismatch/main.aivi",
        "milestone-2/invalid/source-option-contract-parameter-signal-mismatch/main.aivi",
        "milestone-2/invalid/source-option-imported-binding-mismatch/main.aivi",
        "milestone-2/invalid/source-option-constructor-mismatch/main.aivi",
        "milestone-2/invalid/source-option-constructor-application-mismatch/main.aivi",
        "milestone-2/invalid/source-option-list-element-mismatch/main.aivi",
        "milestone-2/invalid/custom-source-provider-option-type-mismatch/main.aivi",
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before source option value validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("source-option-type-mismatch"))),
            "expected {path} to report source-option-type-mismatch, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_custom_source_provider_contract_invalid_fixtures() {
    for (path, code_name) in [
        (
            "milestone-2/invalid/custom-source-provider-unknown-option/main.aivi",
            "unknown-source-option",
        ),
        (
            "milestone-2/invalid/custom-source-provider-argument-count-mismatch/main.aivi",
            "source-argument-count-mismatch",
        ),
        (
            "milestone-2/invalid/custom-source-provider-argument-type-mismatch/main.aivi",
            "source-argument-type-mismatch",
        ),
        (
            "milestone-2/invalid/custom-source-provider-unsupported-schema-type/main.aivi",
            "unsupported-source-provider-contract-type",
        ),
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before custom provider contract validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
            "expected {path} to report {code_name}, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_rejects_recurrence_wakeup_invalid_fixtures() {
    {
        let path = "milestone-2/invalid/missing-recurrence-wakeup/main.aivi";
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before recurrence wakeup validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code
                        == Some(super::code("missing-recurrence-wakeup"))),
                "expected {path} to report missing-recurrence-wakeup, got diagnostics: {:?}",
                report.diagnostics()
            );
    }
}

#[test]
fn resolved_validation_rejects_bodyful_source_signals() {
    for path in [
        "milestone-2/invalid/custom-source-recurrence-missing-wakeup/main.aivi",
        "milestone-2/invalid/request-recurrence-missing-wakeup/main.aivi",
    ] {
        let lowered = lower_fixture(path);
        assert!(
            !lowered.has_errors(),
            "expected {path} to lower cleanly before source validation, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.diagnostics().iter().any(|diagnostic| diagnostic.code
                == Some(super::code("source-signals-must-be-bodyless"))),
            "expected {path} to report source-signals-must-be-bodyless, got diagnostics: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn resolved_validation_accepts_request_sources_with_retry_policy_and_accumulate() {
    let lowered = lower_text(
        "request_source_with_retry_and_scan.aivi",
        r#"
type HttpError =
  | Timeout

type User = {
    id: Int
}

domain Retry over Int = {
    suffix times : Int = value => Retry value
}
fun keepCount:Int = response:(Result HttpError (List User)) current:Int=>    current

@source http.get "/users" with {
    retry: 3times
}
signal responses : Signal (Result HttpError (List User))

signal retried : Signal Int =
    responses
     +|> 0 keepCount
"#,
    );
    assert!(
        !lowered.has_errors(),
        "request source with retry and accumulate should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "request source with retry and accumulate should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_reactive_source_option_payloads() {
    let lowered = lower_text(
        "reactive_source_option_payloads.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}
signal enabled : Signal Bool =
    True

signal jitterValue : Signal Duration =
    5sec

@source timer.every 120 with {
    immediate: enabled,
    activeWhen: enabled,
    jitter: jitterValue
}
signal tick : Signal Unit
"#,
    );
    assert!(
        !lowered.has_errors(),
        "reactive source option payloads should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "reactive source option payloads should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_allows_active_when_feedback_through_accumulate() {
    let lowered = lower_text(
        "active_when_feedback.aivi",
        r#"
fun bump:Int = tick:Unit current:Int=>    current + 1

fun keepRunning:Bool = count:Int=>    count < 3

signal count : Signal Int =
    tick
    +|> 0 bump

signal enabled : Signal Bool =
    count
    |> keepRunning

@source timer.every 120 with {
    activeWhen: enabled
}
signal tick : Signal Unit
"#,
    );
    assert!(
        !lowered.has_errors(),
        "activeWhen feedback example should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "activeWhen feedback loop should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_custom_sources_feeding_accumulate_signals() {
    let lowered = lower_fixture("milestone-2/valid/custom-source-recurrence-wakeup/main.aivi");
    assert!(
        !lowered.has_errors(),
        "custom source accumulate fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "custom source accumulate fixture should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );

    let update_events = find_signal(lowered.module(), "updateEvents");
    let metadata = update_events
        .source_metadata
        .as_ref()
        .expect("bodyless custom source signal should still carry source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Custom("custom.feed".into())
    );
    assert!(
        metadata.is_reactive,
        "reactive custom source arguments should mark the source metadata as reactive"
    );
    assert_eq!(
        metadata.custom_contract, None,
        "surface lowering should not invent custom provider contract metadata"
    );
    assert_eq!(
        signal_dependency_names(lowered.module(), update_events),
        vec!["refresh".to_owned()],
        "custom source metadata should still track provider-independent reactive dependencies"
    );
    let updates = find_signal(lowered.module(), "updates");
    assert_eq!(
        signal_dependency_names(lowered.module(), updates),
        vec!["updateEvents".to_owned()],
        "accumulate-derived signals should depend on the raw source signal rather than provider inputs"
    );
}

#[test]
fn resolves_provider_contract_declarations_onto_source_use_sites() {
    let lowered =
        lower_fixture("milestone-2/valid/source-provider-contract-declarations/main.aivi");
    assert!(
        !lowered.has_errors(),
        "provider contract declaration fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let contract = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::SourceProviderContract(item) => Some(item),
            _ => None,
        })
        .expect("expected to find provider contract item");
    assert_eq!(
        contract.provider,
        SourceProviderRef::Custom("custom.feed".into())
    );
    assert_eq!(
        contract.contract.recurrence_wakeup,
        Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger)
    );
    assert_eq!(contract.contract.arguments.len(), 1);
    assert_eq!(contract.contract.arguments[0].name.text(), "path");
    assert_eq!(contract.contract.options.len(), 2);
    assert_eq!(contract.contract.options[0].name.text(), "timeout");
    assert_eq!(contract.contract.options[1].name.text(), "mode");
    assert_eq!(contract.contract.operations.len(), 1);
    assert_eq!(contract.contract.operations[0].name.text(), "read");
    assert_eq!(contract.contract.commands.len(), 1);
    assert_eq!(contract.contract.commands[0].name.text(), "delete");

    let updates = find_signal(lowered.module(), "updates");
    let metadata = updates
        .source_metadata
        .as_ref()
        .expect("source-backed signal should keep source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Custom("custom.feed".into())
    );
    assert_eq!(
        metadata.custom_contract,
        Some(crate::CustomSourceContractMetadata {
            recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            arguments: contract.contract.arguments.clone(),
            options: contract.contract.options.clone(),
            operations: contract.contract.operations.clone(),
            commands: contract.contract.commands.clone(),
        }),
        "same-module provider declarations should resolve onto matching custom @source use sites"
    );
}

#[test]
fn duplicate_provider_contracts_do_not_attach_ambiguous_use_site_metadata() {
    let lowered = lower_text(
        "duplicate_provider_contract_use_site.aivi",
        r#"
provider custom.feed
    wakeup: timer

provider custom.feed
    wakeup: backoff

@source custom.feed
signal updates : Signal Int
"#,
    );
    assert!(
        lowered.has_errors(),
        "duplicate provider contract test should still report lowering errors"
    );

    let updates = find_signal(lowered.module(), "updates");
    let metadata = updates
        .source_metadata
        .as_ref()
        .expect("custom source should still carry source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Custom("custom.feed".into())
    );
    assert_eq!(
        metadata.custom_contract, None,
        "ambiguous provider contract lookup must not attach arbitrary custom wakeup metadata"
    );
}

#[test]
fn provider_contract_resolution_is_order_independent_within_module() {
    let lowered = lower_text(
        "provider_contract_resolution_order.aivi",
        r#"
@source custom.feed
signal updates : Signal Int

provider custom.feed
    wakeup: timer
"#,
    );
    assert!(
        !lowered.has_errors(),
        "same-module provider declarations should resolve regardless of source order: {:?}",
        lowered.diagnostics()
    );

    let updates = find_signal(lowered.module(), "updates");
    let metadata = updates
        .source_metadata
        .as_ref()
        .expect("custom source should still carry source metadata");
    assert_eq!(
        metadata.custom_contract,
        Some(crate::CustomSourceContractMetadata {
            recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::Timer),
            arguments: Vec::new(),
            options: Vec::new(),
            operations: Vec::new(),
            commands: Vec::new(),
        }),
        "provider contract resolution should use the module namespace rather than declaration order"
    );
}

#[test]
fn provider_contract_declarations_report_builtin_keys_and_invalid_fields() {
    let lowered = lower_text(
        "provider_contract_errors.aivi",
        r#"
provider http.get
    wakeup: surprise
    mode: manual
    wakeup: timer
"#,
    );
    let codes = lowered
        .diagnostics()
        .iter()
        .filter_map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(
        codes.contains(&super::code("builtin-source-provider-contract")),
        "expected built-in provider contract diagnostic, got diagnostics: {:?}",
        lowered.diagnostics()
    );
    assert!(
        codes.contains(&super::code("unknown-source-provider-contract-wakeup")),
        "expected unknown wakeup diagnostic, got diagnostics: {:?}",
        lowered.diagnostics()
    );
    assert!(
        codes.contains(&super::code("unknown-source-provider-contract-field")),
        "expected unknown field diagnostic, got diagnostics: {:?}",
        lowered.diagnostics()
    );
    assert!(
        codes.contains(&super::code("duplicate-source-provider-contract-field")),
        "expected duplicate wakeup diagnostic, got diagnostics: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn provider_contract_declarations_report_duplicate_schema_names() {
    let lowered = lower_text(
        "provider_contract_duplicate_schemas.aivi",
        r#"
provider custom.feed
    argument path: Text
    argument path: Int
    option timeout: Text
    option timeout: Bool
"#,
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(super::code("duplicate-source-provider-contract-field"))
            })
            .count()
            >= 2,
        "expected duplicate schema diagnostics, got diagnostics: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn provider_contract_declarations_report_duplicate_capability_member_names() {
    let lowered = lower_text(
        "provider_contract_duplicate_capability_members.aivi",
        r#"
provider custom.feed
    operation read : Text -> Signal Text
    command read : Text -> Task Text Unit
"#,
    );
    assert!(
        lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(super::code("duplicate-source-provider-contract-field"))
        }),
        "expected duplicate capability member diagnostic, got diagnostics: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn provider_contract_metadata_allows_nonreactive_recurrence() {
    let lowered = lower_fixture("milestone-2/valid/custom-source-provider-wakeup/main.aivi");
    assert!(
        !lowered.has_errors(),
        "provider-declared custom source wakeup fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let retry_events = find_signal(lowered.module(), "retryEvents");
    let metadata = retry_events
        .source_metadata
        .as_ref()
        .expect("provider-defined source signal should carry source metadata");
    assert!(
        !metadata.is_reactive,
        "provider-declared recurrence fixture should stay non-reactive"
    );
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Custom("custom.feed".into())
    );
    assert_eq!(
        metadata.custom_contract,
        Some(crate::CustomSourceContractMetadata {
            recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            arguments: Vec::new(),
            options: Vec::new(),
            operations: Vec::new(),
            commands: Vec::new(),
        }),
        "matching provider contracts should populate custom wakeup metadata before validation"
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "resolved custom provider metadata should unblock recurrence validation, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn manual_custom_source_contract_metadata_rejects_builtin_providers() {
    let lowered = lower_fixture("milestone-1/valid/sources/source_declarations.aivi");
    assert!(
        !lowered.has_errors(),
        "built-in source fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let mut module = lowered.module().clone();
    let signal_id = module
        .root_items()
        .iter()
        .copied()
        .find(|item_id| {
            matches!(
                &module.items()[*item_id],
                Item::Signal(item) if item.name.text() == "tick"
            )
        })
        .expect("expected to find `tick` signal item");
    let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
        panic!("expected `tick` item to stay a signal");
    };
    signal
        .source_metadata
        .as_mut()
        .expect("built-in source should carry source metadata")
        .custom_contract = Some(crate::CustomSourceContractMetadata {
        recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
        arguments: Vec::new(),
        options: Vec::new(),
        operations: Vec::new(),
        commands: Vec::new(),
    });

    let report = module.validate(ValidationMode::RequireResolvedNames);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("invalid-custom-source-wakeup"))),
        "built-in sources should reject injected custom contract metadata, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn manual_custom_source_contract_metadata_rejects_invalid_provider_shapes() {
    let lowered = lower_fixture("milestone-2/invalid/source-provider-without-variant/main.aivi");
    assert!(
        lowered.has_errors(),
        "invalid provider fixture should still report a lowering error"
    );

    let mut module = lowered.module().clone();
    let signal_id = module
        .root_items()
        .iter()
        .copied()
        .find(|item_id| {
            matches!(
                &module.items()[*item_id],
                Item::Signal(item) if item.name.text() == "users"
            )
        })
        .expect("expected to find `users` signal item");
    let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
        panic!("expected `users` item to stay a signal");
    };
    signal
        .source_metadata
        .as_mut()
        .expect("invalid provider shape should still preserve source metadata")
        .custom_contract = Some(crate::CustomSourceContractMetadata {
        recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
        arguments: Vec::new(),
        options: Vec::new(),
        operations: Vec::new(),
        commands: Vec::new(),
    });

    let report = module.validate(ValidationMode::Structural);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("invalid-custom-source-wakeup"))),
        "malformed provider paths should reject injected custom contract metadata, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_explicit_recurrence_wakeup_fixture() {
    let lowered = lower_fixture("milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi");
    assert!(
        !lowered.has_errors(),
        "explicit recurrence wakeup fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "explicit recurrence wakeup fixture should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn resolved_validation_accepts_pipe_stage_memo_fixture() {
    let lowered = lower_fixture("milestone-2/valid/pipe-stage-memos/main.aivi");
    assert!(
        !lowered.has_errors(),
        "pipe stage memo fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "pipe stage memo fixture should validate cleanly, got diagnostics: {:?}",
        report.diagnostics()
    );

    let transformed = find_value(lowered.module(), "transformed");
    let ExprKind::Pipe(transformed_pipe) = &lowered.module().exprs()[transformed.body].kind else {
        panic!("expected transformed to lower as a pipe expression");
    };
    assert!(matches!(
        transformed_pipe.stages.first().kind,
        PipeStageKind::Transform { .. }
    ));
    assert!(transformed_pipe.stages.first().subject_memo.is_some());
    assert!(transformed_pipe.stages.first().result_memo.is_some());

    let case_memo = find_value(lowered.module(), "caseMemo");
    let ExprKind::Pipe(case_pipe) = &lowered.module().exprs()[case_memo.body].kind else {
        panic!("expected caseMemo to lower as a pipe expression");
    };
    assert!(matches!(
        case_pipe.stages.first().kind,
        PipeStageKind::Case { .. }
    ));
    assert!(case_pipe.stages.first().subject_memo.is_some());
    assert!(case_pipe.stages.first().result_memo.is_some());
    assert!(
        case_pipe
            .stages
            .iter()
            .skip(1)
            .all(|stage| stage.result_memo.is_none())
    );

    let truthy_memo = find_value(lowered.module(), "truthyMemo");
    let ExprKind::Pipe(truthy_pipe) = &lowered.module().exprs()[truthy_memo.body].kind else {
        panic!("expected truthyMemo to lower as a pipe expression");
    };
    assert!(matches!(
        truthy_pipe.stages.first().kind,
        PipeStageKind::Truthy { .. }
    ));
    assert!(truthy_pipe.stages.first().subject_memo.is_some());
    assert!(truthy_pipe.stages.first().result_memo.is_some());
    assert!(
        truthy_pipe
            .stages
            .iter()
            .skip(1)
            .all(|stage| stage.result_memo.is_none())
    );

    let delayed_value = find_signal(lowered.module(), "delayedValue");
    let delayed_body = delayed_value
        .body
        .expect("delayedValue should lower to a pipe expression");
    let ExprKind::Pipe(delayed_pipe) = &lowered.module().exprs()[delayed_body].kind else {
        panic!("expected delayedValue to lower as a pipe expression");
    };
    assert!(matches!(
        delayed_pipe.stages.first().kind,
        PipeStageKind::Delay { .. }
    ));
    assert!(delayed_pipe.stages.first().result_memo.is_some());

    let accumulated = find_signal(lowered.module(), "accumulated");
    let accumulated_body = accumulated
        .body
        .expect("accumulated should lower to a pipe expression");
    let ExprKind::Pipe(accumulated_pipe) = &lowered.module().exprs()[accumulated_body].kind else {
        panic!("expected accumulated to lower as a pipe expression");
    };
    assert!(matches!(
        accumulated_pipe.stages.first().kind,
        PipeStageKind::Accumulate { .. }
    ));
    assert!(accumulated_pipe.stages.first().subject_memo.is_some());
    assert!(accumulated_pipe.stages.first().result_memo.is_some());
}

#[test]
fn lowers_recurrence_wakeup_decorators_into_typed_payloads() {
    let lowered = lower_fixture("milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi");
    assert!(
        !lowered.has_errors(),
        "explicit recurrence wakeup fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let polled = find_signal(lowered.module(), "polled");
    let polled_decorator = &lowered.module().decorators()[polled.header.decorators[0]];
    match &polled_decorator.payload {
        DecoratorPayload::RecurrenceWakeup(wakeup) => {
            assert_eq!(wakeup.kind, RecurrenceWakeupDecoratorKind::Timer);
            assert!(matches!(
                lowered.module().exprs()[wakeup.witness].kind,
                ExprKind::Integer(_) | ExprKind::SuffixedInteger(_)
            ));
        }
        other => panic!(
            "expected `polled` to carry a typed recurrence wakeup decorator, found {other:?}"
        ),
    }

    let Item::Value(retried) = find_named_item(lowered.module(), "retried") else {
        panic!("expected `retried` to be a value item");
    };
    let retried_decorator = &lowered.module().decorators()[retried.header.decorators[0]];
    match &retried_decorator.payload {
        DecoratorPayload::RecurrenceWakeup(wakeup) => {
            assert_eq!(wakeup.kind, RecurrenceWakeupDecoratorKind::Backoff);
            assert!(matches!(
                lowered.module().exprs()[wakeup.witness].kind,
                ExprKind::Integer(_) | ExprKind::SuffixedInteger(_)
            ));
        }
        other => panic!(
            "expected `retried` to carry a typed recurrence wakeup decorator, found {other:?}"
        ),
    }
}

#[test]
fn recurrence_wakeup_decorators_reject_invalid_shapes_and_source_mix() {
    let lowered = lower_text(
        "invalid_recurrence_wakeup_decorators.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}
domain Retry over Int = {
    suffix times : Int = value => Retry value
}
fun step = x=>    x

@recur.timer
signal bare : Signal Int =
    0
     @|> step
     <|@ step

@source http.get "/users"
@recur.backoff 3times
signal mixed : Signal Int =
    0
     @|> step
     <|@ step

@recur.timer 5sec
@recur.backoff 3times
value duplicate : Task Int Int =
    0
     @|> step
     <|@ step
"#,
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(super::code("invalid-recurrence-wakeup-decorator"))),
        "expected invalid recurrence wakeup shape diagnostic, got {:?}",
        lowered.diagnostics()
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(super::code("invalid-source-recurrence-wakeup"))),
        "expected source/non-source recurrence wakeup conflict diagnostic, got {:?}",
        lowered.diagnostics()
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(super::code("duplicate-recurrence-wakeup-decorator"))),
        "expected duplicate recurrence wakeup decorator diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn preserves_bodyless_source_signals_and_provider_paths() {
    let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
    assert!(
        !lowered.has_errors(),
        "source-decorator fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let users = find_signal(lowered.module(), "users");
    assert!(
        users.body.is_none(),
        "source-backed signals should stay bodyless in HIR"
    );
    let metadata = users
        .source_metadata
        .as_ref()
        .expect("source-backed signal should carry source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Builtin(BuiltinSourceProvider::HttpGet),
        "source metadata should preserve built-in provider identity"
    );
    assert_eq!(
        metadata.custom_contract, None,
        "built-in providers should never attach custom-provider contract hooks"
    );
    assert!(
        metadata.is_reactive,
        "interpolated source arguments should mark the source as reactive"
    );
    assert_eq!(
        signal_item_names(lowered.module(), &metadata.signal_dependencies),
        vec!["apiHost".to_owned(), "users#trigger".to_owned()],
        "request-like source metadata should include both the reactive argument dependency and the synthesized trigger helper"
    );
    assert_eq!(
        users.signal_dependencies, metadata.signal_dependencies,
        "source-backed signals should expose the same dependency set at the signal boundary"
    );
    let users_decorator = lowered.module().decorators()[users.header.decorators[0]].clone();
    match users_decorator.payload {
        DecoratorPayload::Source(source) => {
            assert_eq!(
                source.provider.as_ref().map(path_text).as_deref(),
                Some("http.get"),
                "@source provider path should be preserved exactly"
            );
        }
        other => panic!("expected source decorator payload, found {other:?}"),
    }

    let tick = find_signal(lowered.module(), "tick");
    assert!(
        tick.body.is_none(),
        "bodyless timer source signal should stay bodyless"
    );
    let metadata = tick
        .source_metadata
        .as_ref()
        .expect("timer source should still carry source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Builtin(BuiltinSourceProvider::TimerEvery)
    );
    assert_eq!(
        metadata.custom_contract, None,
        "built-in source metadata should not use the custom-provider wakeup hook"
    );
    assert!(
        !metadata.is_reactive,
        "non-reactive source arguments should stay non-reactive"
    );
    assert!(
        metadata.signal_dependencies.is_empty(),
        "non-reactive sources should not record signal dependencies"
    );
    assert_eq!(
        tick.signal_dependencies, metadata.signal_dependencies,
        "non-reactive source signals should expose an empty dependency set"
    );
}

#[test]
fn classifies_source_lifecycle_dependency_roles() {
    let lowered = lower_text(
        "source_lifecycle_dependency_roles.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}
provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool

signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval : Signal Duration = 5sec
signal path = "/tmp/demo.txt"

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int

@source custom.feed path with {
    activeWhen: enabled
}
signal updates : Signal Int
"#,
    );
    assert!(
        !lowered.has_errors(),
        "source lifecycle dependency role fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let users = find_signal(lowered.module(), "users");
    let metadata = users
        .source_metadata
        .as_ref()
        .expect("built-in source should carry source metadata");
    assert_eq!(
        signal_dependency_names(lowered.module(), users),
        vec![
            "apiHost".to_owned(),
            "refresh".to_owned(),
            "enabled".to_owned(),
            "pollInterval".to_owned()
        ]
    );
    assert_eq!(
        metadata.lifecycle_dependencies.merged(),
        metadata.signal_dependencies,
        "lifecycle dependency roles should merge back into the overall source dependency set"
    );
    assert_eq!(
        signal_item_names(
            lowered.module(),
            &metadata.lifecycle_dependencies.reconfiguration
        ),
        vec!["apiHost".to_owned(), "pollInterval".to_owned()]
    );
    assert_eq!(
        signal_item_names(
            lowered.module(),
            &metadata.lifecycle_dependencies.explicit_triggers
        ),
        vec!["refresh".to_owned()]
    );
    assert_eq!(
        signal_item_names(
            lowered.module(),
            &metadata.lifecycle_dependencies.active_when
        ),
        vec!["enabled".to_owned()]
    );

    let updates = find_signal(lowered.module(), "updates");
    let metadata = updates
        .source_metadata
        .as_ref()
        .expect("custom source should carry source metadata");
    assert_eq!(
        signal_item_names(
            lowered.module(),
            &metadata.lifecycle_dependencies.reconfiguration
        ),
        vec!["enabled".to_owned(), "path".to_owned()]
    );
    assert!(
        metadata.lifecycle_dependencies.explicit_triggers.is_empty(),
        "custom sources must not invent built-in trigger roles"
    );
    assert!(
        metadata.lifecycle_dependencies.active_when.is_empty(),
        "custom sources must not invent built-in activeWhen roles"
    );
}

#[test]
fn manual_source_lifecycle_metadata_inconsistency_is_rejected() {
    let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
    assert!(
        !lowered.has_errors(),
        "source lifecycle validation fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let mut module = lowered.module().clone();
    let signal_id = module
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(&module.items()[*item_id], Item::Signal(item) if item.name.text() == "users")
            })
            .expect("expected to find `users` signal item");
    let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
        panic!("expected `users` item to stay a signal");
    };
    signal
        .source_metadata
        .as_mut()
        .expect("source-backed signal should carry source metadata")
        .lifecycle_dependencies
        .reconfiguration
        .clear();

    let report = module.validate(ValidationMode::Structural);
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(super::code("inconsistent-source-lifecycle-dependencies"))
        }),
        "inconsistent source lifecycle dependency roles should be rejected, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn lowers_structured_text_interpolation_in_source_arguments() {
    let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
    assert!(
        !lowered.has_errors(),
        "source-decorator fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let users = find_signal(lowered.module(), "users");
    let users_decorator = &lowered.module().decorators()[users.header.decorators[0]];
    let DecoratorPayload::Source(source) = &users_decorator.payload else {
        panic!("expected source decorator payload");
    };
    let argument = source.arguments[0];
    let ExprKind::Text(text) = &lowered.module().exprs()[argument].kind else {
        panic!("expected interpolated text argument");
    };
    assert_eq!(text.segments.len(), 2);
    match &text.segments[0] {
        TextSegment::Interpolation(interpolation) => {
            let ExprKind::Name(reference) = &lowered.module().exprs()[interpolation.expr].kind
            else {
                panic!("expected interpolation hole to lower as a name expression");
            };
            assert_eq!(
                path_text(&reference.path),
                "apiHost",
                "interpolation should preserve the embedded expression"
            );
            assert!(
                matches!(
                    reference.resolution,
                    ResolutionState::Resolved(TermResolution::Item(_))
                ),
                "interpolation names should resolve like ordinary expressions"
            );
        }
        other => panic!("expected leading interpolation segment, got {other:?}"),
    }
    match &text.segments[1] {
        TextSegment::Text(fragment) => assert_eq!(&*fragment.raw, "/users"),
        other => panic!("expected trailing text segment, got {other:?}"),
    }
}

#[test]
fn tracks_signal_dependencies_for_ordinary_derived_signals() {
    let lowered = lower_fixture("milestone-2/valid/applicative-clusters/main.aivi");
    assert!(
        !lowered.has_errors(),
        "applicative cluster fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let validated_user = find_signal(lowered.module(), "validatedUser");
    assert_eq!(
        signal_dependency_names(lowered.module(), validated_user),
        vec![
            "nameText".to_owned(),
            "emailText".to_owned(),
            "ageValue".to_owned(),
        ],
        "derived signals should collect the union of referenced signal dependencies"
    );
    assert!(
        validated_user.source_metadata.is_none(),
        "ordinary derived signals should not carry source metadata"
    );

    let name_pair = find_signal(lowered.module(), "namePair");
    assert_eq!(
        signal_dependency_names(lowered.module(), name_pair),
        vec!["firstName".to_owned(), "lastName".to_owned()],
        "applicative derived signals should keep deterministic dependency ordering"
    );

    let local_refs = lower_fixture("milestone-2/valid/local-top-level-refs/main.aivi");
    assert!(
        !local_refs.has_errors(),
        "local top-level refs fixture should lower cleanly: {:?}",
        local_refs.diagnostics()
    );
    let next_refresh = find_signal(local_refs.module(), "nextRefresh");
    assert_eq!(
        signal_dependency_names(local_refs.module(), next_refresh),
        vec!["refreshMs".to_owned()],
        "value references must not leak into signal dependency metadata"
    );
}

#[test]
fn tracks_signal_dependencies_through_helper_bodies() {
    let lowered = lower_text(
        "signal-helper-dependencies.aivi",
        r#"signal direction : Signal Int = 1
signal tick : Signal Int = 0
fun stepOnTick:Int = tick:Int => direction
signal game : Signal Int = stepOnTick tick
"#,
    );
    assert!(
        !lowered.has_errors(),
        "helper-body dependency example should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let game = find_signal(lowered.module(), "game");
    assert_eq!(
        signal_dependency_names(lowered.module(), game),
        vec!["direction".to_owned(), "tick".to_owned()],
        "signal dependency metadata should include signal reads hidden behind helper bodies"
    );
}

#[test]
fn tracks_signal_dependencies_through_signal_record_projections() {
    let lowered = lower_text(
        "signal-projection-dependencies.aivi",
        "type Game = { score: Int }\n\
             type State = { game: Game, seenRestartCount: Int }\n\
             signal state : Signal State = { game: { score: 0 }, seenRestartCount: 0 }\n\
             signal game : Signal Game = state.game\n\
             signal score : Signal Int = state.game.score\n",
    );
    assert!(
        !lowered.has_errors(),
        "signal projection dependency example should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "signal projection dependency example should validate cleanly: {:?}",
        report.diagnostics()
    );

    let game = find_signal(lowered.module(), "game");
    assert_eq!(
        signal_dependency_names(lowered.module(), game),
        vec!["state".to_owned()],
        "projecting a field out of a signal record should keep the upstream signal dependency"
    );

    let score = find_signal(lowered.module(), "score");
    assert_eq!(
        signal_dependency_names(lowered.module(), score),
        vec!["state".to_owned()],
        "nested signal record projections should still trace back to the original upstream signal"
    );
}

#[test]
fn lowers_from_signal_fanout_into_ordinary_signals() {
    let lowered = lower_text(
        "from-signal-fanout.aivi",
        "type Status =\n\
             \x20 | Running\n\
             \x20 | GameOver\n\
             type State = { status: Status, score: Int }\n\
             signal state : Signal State = { status: Running, score: 0 }\n\
             from state = {\n\
             \x20\x20\x20\x20score: .score\n\
             \x20\x20\x20\x20gameOver: .status\n\
             \x20\x20\x20\x20\x20\x20\x20\x20||> Running -> False\n\
             \x20\x20\x20\x20\x20\x20\x20\x20||> GameOver -> True\n\
             }\n",
    );
    assert!(
        !lowered.has_errors(),
        "from signal fanout example should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "from signal fanout example should validate cleanly: {:?}",
        report.diagnostics()
    );

    let score = find_signal(lowered.module(), "score");
    assert_eq!(
        signal_dependency_names(lowered.module(), score),
        vec!["state".to_owned()],
        "fan-out entries should lower to ordinary derived signals over the shared source"
    );

    let game_over = find_signal(lowered.module(), "gameOver");
    assert_eq!(
        signal_dependency_names(lowered.module(), game_over),
        vec!["state".to_owned()],
        "pipe-case fan-out entries should preserve the upstream signal dependency"
    );
}

#[test]
fn lowers_parameterized_from_entries_into_selector_functions() {
    let lowered = lower_text(
        "from-parameterized-selectors.aivi",
        "type State = { score: Int, ready: Bool }\n\
             signal state : Signal State = { score: 0, ready: True }\n\
             from state = {\n\
             \x20\x20\x20\x20type Bool\n\
             \x20\x20\x20\x20readyNow: .ready\n\
             \x20\x20\x20\x20type Int -> Bool\n\
             \x20\x20\x20\x20atLeast threshold: .score >= threshold\n\
             }\n\
             signal thresholdMet : Signal Bool = atLeast 0\n",
    );
    assert!(
        !lowered.has_errors(),
        "parameterized from-entry example should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "parameterized from-entry example should validate cleanly: {:?}",
        report.diagnostics()
    );

    let ready_now = find_signal(lowered.module(), "readyNow");
    assert_eq!(
        signal_dependency_names(lowered.module(), ready_now),
        vec!["state".to_owned()],
        "zero-parameter from entries should keep lowering to derived signals"
    );

    let at_least = match find_named_item(lowered.module(), "atLeast") {
        Item::Function(item) => item,
        other => panic!("expected `atLeast` to lower as a function item, found {other:?}"),
    };
    assert_eq!(at_least.parameters.len(), 1);

    let parameter_annotation = at_least.parameters[0]
        .annotation
        .expect("parameterized from entry should synthesize its parameter annotation");
    match &lowered.module().types()[parameter_annotation].kind {
        TypeKind::Name(reference) => assert!(
            matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Int))
            ),
            "expected selector parameter annotation to resolve to builtin `Int`, found {:?}",
            reference.resolution.as_ref()
        ),
        other => panic!("expected builtin `Int` parameter annotation, found {other:?}"),
    }

    let result_annotation = at_least
        .annotation
        .expect("parameterized from entry should keep its synthesized reactive result type");
    let TypeKind::Apply { callee, arguments } = &lowered.module().types()[result_annotation].kind
    else {
        panic!("expected selector result to be wrapped in `Signal`");
    };
    assert_eq!(arguments.len(), 1);
    match &lowered.module().types()[*arguments.first()].kind {
        TypeKind::Name(reference) => assert!(
            matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Bool))
            ),
            "expected selector payload to resolve to builtin `Bool`, found {:?}",
            reference.resolution.as_ref()
        ),
        other => panic!("expected builtin `Bool` payload, found {other:?}"),
    }
    match &lowered.module().types()[*callee].kind {
        TypeKind::Name(reference) => assert!(
            matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal))
            ),
            "expected selector result wrapper to resolve to builtin `Signal`, found {:?}",
            reference.resolution.as_ref()
        ),
        other => panic!("expected builtin `Signal` callee, found {other:?}"),
    }
}

#[test]
fn normalizes_expression_headed_clusters_into_spines() {
    let lowered = lower_text(
        "expression-headed-clusters.aivi",
        "type NamePair = NamePair Text Text\n\
             signal firstName = \"Ada\"\n\
             signal lastName = \"Lovelace\"\n\
             signal headedPair =\n\
              firstName\n\
               &|> lastName\n\
                |> NamePair\n\
             signal headedTuple =\n\
              firstName\n\
               &|> lastName\n",
    );
    assert!(
        !lowered.has_errors(),
        "expression-headed clusters should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let headed_pair = find_signal(lowered.module(), "headedPair");
    let pair_body = headed_pair
        .body
        .expect("headedPair should lower to a cluster expression");
    let ExprKind::Cluster(pair_cluster_id) = &lowered.module().exprs()[pair_body].kind else {
        panic!("expected headedPair to lower as a cluster expression");
    };
    let pair_cluster = &lowered.module().clusters()[*pair_cluster_id];
    assert_eq!(
        pair_cluster.presentation,
        ClusterPresentation::ExpressionHeaded,
        "expression-headed surface form should stay visible in HIR"
    );
    let pair_spine = pair_cluster.normalized_spine();
    let pair_arguments = pair_spine
        .apply_arguments()
        .map(|expr_id| match &lowered.module().exprs()[expr_id].kind {
            ExprKind::Name(reference) => path_text(&reference.path),
            other => {
                panic!("expected normalized cluster argument to stay a name, found {other:?}")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pair_arguments,
        vec!["firstName".to_owned(), "lastName".to_owned()],
        "normalized applicative spines should preserve cluster member order"
    );
    match pair_spine.pure_head() {
        ApplicativeSpineHead::Expr(expr_id) => match &lowered.module().exprs()[expr_id].kind {
            ExprKind::Name(reference) => assert_eq!(path_text(&reference.path), "NamePair"),
            other => panic!("expected explicit spine head to stay a name, found {other:?}"),
        },
        other => panic!("expected explicit applicative head, found {other:?}"),
    }

    let headed_tuple = find_signal(lowered.module(), "headedTuple");
    let tuple_body = headed_tuple
        .body
        .expect("headedTuple should lower to a cluster expression");
    let ExprKind::Cluster(tuple_cluster_id) = &lowered.module().exprs()[tuple_body].kind else {
        panic!("expected headedTuple to lower as a cluster expression");
    };
    match lowered.module().clusters()[*tuple_cluster_id]
        .normalized_spine()
        .pure_head()
    {
        ApplicativeSpineHead::TupleConstructor(arity) => assert_eq!(arity.get(), 2),
        other => panic!("expected implicit tuple applicative head, found {other:?}"),
    }
}

#[test]
fn allows_nested_pipe_subjects_inside_clusters() {
    let lowered = lower_text(
        "nested-cluster-pipe-subject.aivi",
        "type NamePair = NamePair Text Text\n\
             signal firstName = \"Ada\"\n\
             signal lastName = \"Lovelace\"\n\
             signal ok =\n\
              firstName\n\
               &|> (lastName |> .display)\n\
                |> NamePair\n",
    );
    assert!(
        !lowered.has_errors(),
        "nested pipes with their own heads should remain legal inside clusters: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn rejects_interpolated_pattern_text() {
    let lowered = lower_text(
        "interpolated-pattern-text.aivi",
        "value subject = \"Ada\"\nvalue result = subject ||> \"{subject}\" -> 1\n",
    );
    assert!(
        lowered.has_errors(),
        "interpolated pattern text should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("interpolated-pattern-text"))),
        "expected interpolated-pattern-text diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "invalid interpolated-pattern-text fixture should keep structural HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_unfinished_cluster_continuations() {
    let lowered = lower_fixture("milestone-1/invalid/cluster_unfinished_gate.aivi");
    assert!(
        lowered.has_errors(),
        "unfinished applicative clusters should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("illegal-unfinished-cluster"))),
        "expected illegal-unfinished-cluster diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "unfinished cluster errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_ambient_projections_inside_clusters() {
    let lowered = lower_fixture("milestone-2/invalid/cluster-ambient-projection/main.aivi");
    assert!(
        lowered.has_errors(),
        "ambient projections should be rejected inside applicative clusters"
    );
    assert!(
        lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(super::code("illegal-cluster-ambient-projection"))
        }),
        "expected illegal-cluster-ambient-projection diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "cluster ambient-projection errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_duplicate_source_options() {
    let lowered = lower_fixture("milestone-2/invalid/source-duplicate-option/main.aivi");
    assert!(
        lowered.has_errors(),
        "duplicate source options should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-source-option"))),
        "expected duplicate-source-option diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "duplicate source options should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_source_provider_without_variant() {
    let lowered = lower_fixture("milestone-2/invalid/source-provider-without-variant/main.aivi");
    assert!(
        lowered.has_errors(),
        "source providers without variants should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("invalid-source-provider"))),
        "expected invalid-source-provider diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "source provider shape errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
    let users = find_signal(lowered.module(), "users");
    let metadata = users
        .source_metadata
        .as_ref()
        .expect("invalid source provider fixture should still preserve source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::InvalidShape("http".into())
    );
}

#[test]
fn lowers_source_capability_handles_into_existing_source_and_task_paths() {
    let lowered = lower_text(
        "source_capability_handles.aivi",
        r#"
type FsSource = Unit
type FsError = Text

signal projectRoot : Signal Text = "/tmp/demo"

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
value cleanup = files.delete "cache.txt"
"#,
    );
    assert!(
        !lowered.has_errors(),
        "capability handle lowering should not introduce front-end diagnostics: {:?}",
        lowered.diagnostics()
    );

    let files = find_signal(lowered.module(), "files");
    assert!(
        files.is_source_capability_handle,
        "bodyless `@source fs` anchors should lower as capability handles"
    );
    assert!(
        files.source_metadata.is_none(),
        "capability handles must not produce executable source metadata"
    );
    assert!(
        crate::exports::exports(lowered.module())
            .find("files")
            .is_none(),
        "capability handles are compile-time anchors and should not be exported as runtime signals"
    );

    let config = find_signal(lowered.module(), "config");
    assert!(
        config.body.is_none(),
        "signal capability operations should lower into bodyless source bindings"
    );
    let metadata = config
        .source_metadata
        .as_ref()
        .expect("lowered capability source should carry ordinary source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Builtin(aivi_typing::BuiltinSourceProvider::FsRead)
    );
    assert_eq!(
        signal_dependency_names(lowered.module(), config),
        vec!["projectRoot".to_owned(), "config#trigger".to_owned()],
        "capability-lowered request sources should depend on both the inherited handle root and the synthesized trigger helper"
    );

    let cleanup = find_value(lowered.module(), "cleanup");
    let ExprKind::Apply { callee, arguments } = &lowered.module().exprs()[cleanup.body].kind else {
        panic!("expected cleanup to lower into an intrinsic application");
    };
    let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
        panic!("expected cleanup callee to be a resolved intrinsic");
    };
    assert_eq!(
        reference.resolution,
        ResolutionState::Resolved(TermResolution::IntrinsicValue(IntrinsicValue::FsDeleteFile))
    );
    let joined_path = *arguments.first();
    let ExprKind::Apply {
        callee: join_callee,
        arguments: join_arguments,
    } = &lowered.module().exprs()[joined_path].kind
    else {
        panic!("expected cleanup path to lower through a synthesized path join");
    };
    let ExprKind::Name(join_reference) = &lowered.module().exprs()[*join_callee].kind else {
        panic!("expected path join callee to be a resolved intrinsic");
    };
    assert_eq!(
        join_reference.resolution,
        ResolutionState::Resolved(TermResolution::IntrinsicValue(IntrinsicValue::PathJoin))
    );
    assert_eq!(
        join_arguments.len(),
        2,
        "path joins should combine the inherited handle root with the member path"
    );
}

#[test]
fn lowers_custom_source_capability_operations_into_member_qualified_custom_sources() {
    let lowered = lower_text(
        "custom_source_capability_operations.aivi",
        r#"
type FeedSource = Unit

signal root = "/tmp/demo"
signal enabled = True

provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool
    operation read : Text -> Signal Int
    command delete : Text -> Task Text Unit

@source custom.feed root with {
    activeWhen: enabled
}
signal feed : FeedSource

signal config : Signal Int = feed.read "config"
"#,
    );
    assert!(
        !lowered.has_errors(),
        "custom capability operations should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let feed = find_signal(lowered.module(), "feed");
    assert!(
        feed.is_source_capability_handle,
        "bodyless custom @source anchors should lower as capability handles"
    );

    let config = find_signal(lowered.module(), "config");
    assert!(
        config.body.is_none(),
        "custom capability operations should lower into bodyless source bindings"
    );
    let metadata = config
        .source_metadata
        .as_ref()
        .expect("lowered custom capability operation should carry source metadata");
    assert_eq!(
        metadata.provider,
        SourceProviderRef::Custom("custom.feed.read".into())
    );
    assert_eq!(
        signal_dependency_names(lowered.module(), config),
        vec!["root".to_owned(), "enabled".to_owned()],
        "custom capability operations should depend on inherited arguments/options, not the handle anchor"
    );
    let contract = metadata
        .custom_contract
        .as_ref()
        .expect("member-qualified custom sources should attach a derived contract");
    assert_eq!(
        contract.arguments.len(),
        2,
        "derived custom source contracts should include both provider arguments and member arguments"
    );
    assert_eq!(
        contract.options.len(),
        1,
        "member-qualified custom sources should preserve provider options"
    );

    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "member-qualified custom sources should validate against the derived contract, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn custom_source_capability_operations_require_signal_bindings() {
    let lowered = lower_text(
        "custom_source_capability_operation_value.aivi",
        r#"
type FeedSource = Unit

provider custom.feed
    operation read : Text -> Signal Int

@source custom.feed
signal feed : FeedSource

value load = feed.read "config"
"#,
    );
    assert!(
        lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(super::code("invalid-source-capability-value-member"))
        }),
        "custom capability operations should reject `value` bindings, got diagnostics: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn lowers_custom_source_capability_commands_into_typed_runtime_imports() {
    let lowered = lower_text(
        "custom_source_capability_command_value.aivi",
        r#"
type FeedSource = Unit

value mode = "sync"

provider custom.feed
    argument root: Text
    option mode: Text
    command delete : Text -> Task Text Unit

@source custom.feed "/tmp/demo" with {
    mode: mode
}
signal feed : FeedSource

value cleanup : Task Text Unit = feed.delete "config"
"#,
    );
    assert!(
        !lowered.has_errors(),
        "custom capability commands should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let cleanup = find_value(lowered.module(), "cleanup");
    let ExprKind::Apply { callee, arguments } = &lowered.module().exprs()[cleanup.body].kind else {
        panic!("custom capability command values should lower into a runtime application");
    };
    let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
        panic!("custom capability command callee should lower to a synthetic import");
    };
    let ResolutionState::Resolved(TermResolution::Import(import_id)) = reference.resolution else {
        panic!("custom capability command callee should resolve through a synthetic import");
    };
    assert_eq!(
        arguments.len(),
        3,
        "custom capability commands should apply inherited provider args, handle options, and member args"
    );
    let import = &lowered.module().imports()[import_id];
    let ImportBindingMetadata::IntrinsicValue {
        value: IntrinsicValue::CustomCapabilityCommand(spec),
        ..
    } = &import.metadata
    else {
        panic!(
            "custom capability command imports should carry the shared custom command intrinsic"
        );
    };
    assert_eq!(spec.provider_key.as_ref(), "custom.feed");
    assert_eq!(spec.command.as_ref(), "delete");
    assert_eq!(
        spec.provider_arguments.as_ref(),
        &["root".into()],
        "custom capability commands should preserve provider argument names"
    );
    assert_eq!(
        spec.options.as_ref(),
        &["mode".into()],
        "custom capability commands should preserve captured handle option names"
    );
    assert_eq!(
        spec.arguments.as_ref(),
        &["arg1".into()],
        "custom capability commands should synthesize stable member argument names"
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "custom capability commands should validate against the synthetic import type, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_unknown_source_options_for_known_providers() {
    let lowered = lower_fixture("milestone-1/invalid/source_unknown_option.aivi");
    assert!(
        lowered.has_errors(),
        "unknown source options on known providers should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("unknown-source-option"))),
        "expected unknown-source-option diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "unknown source option errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_legacy_quantity_source_option_names() {
    let lowered = lower_fixture("milestone-2/invalid/source-legacy-quantity-option/main.aivi");
    assert!(
        lowered.has_errors(),
        "legacy quantity option names should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("unknown-source-option"))),
        "expected unknown-source-option diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "legacy quantity option errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_orphan_recur_steps() {
    let lowered = lower_fixture("milestone-2/invalid/orphan-recur-step/main.aivi");
    assert!(
        lowered.has_errors(),
        "orphan recurrence steps should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("orphan-recur-step"))),
        "expected orphan-recur-step diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "orphan recurrence step errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_unfinished_recurrence_suffixes() {
    let lowered = lower_fixture("milestone-2/invalid/unfinished-recurrence/main.aivi");
    assert!(
        lowered.has_errors(),
        "unfinished recurrence suffixes should be rejected"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("unfinished-recurrence"))),
        "expected unfinished-recurrence diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "unfinished recurrence errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn rejects_recurrence_suffix_continuations() {
    let lowered = lower_fixture("milestone-2/invalid/recurrence-continuation/main.aivi");
    assert!(
        lowered.has_errors(),
        "recurrence suffix continuations should be rejected"
    );
    assert!(
        lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(super::code("illegal-recurrence-continuation"))
        }),
        "expected illegal-recurrence-continuation diagnostic, got {:?}",
        lowered.diagnostics()
    );
    let report = lowered.module().validate(ValidationMode::Structural);
    assert!(
        report.is_ok(),
        "recurrence continuation errors should keep structurally valid HIR: {:?}",
        report.diagnostics()
    );
}

#[test]
fn does_not_double_report_followup_recurrence_starts() {
    let lowered = lower_text(
        "duplicate-recurrence-starts.aivi",
        "fun step = x => x\nvalue broken = 0 @|> step @|> step <|@ step\n",
    );
    assert!(
        lowered.has_errors(),
        "duplicate recurrence starts should still be rejected"
    );
    let unfinished = lowered
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == Some(super::code("unfinished-recurrence")))
        .count();
    let illegal = lowered
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == Some(super::code("illegal-recurrence-continuation"))
        })
        .count();
    assert_eq!(
        unfinished,
        1,
        "expected exactly one unfinished-recurrence diagnostic, got {:?}",
        lowered.diagnostics()
    );
    assert_eq!(
        illegal,
        1,
        "expected exactly one illegal-recurrence-continuation diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn exposes_trailing_recurrence_suffix_views() {
    let lowered = lower_text(
        "recurrence-suffix-view.aivi",
        r#"fun keep = x => x
fun start = x => x
fun step = x => x
signal retried = 0 |> keep | keep @|> start <|@ step <|@ step
"#,
    );
    assert!(
        !lowered.has_errors(),
        "valid recurrence suffixes should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let retried = find_signal(lowered.module(), "retried");
    let body = retried
        .body
        .expect("retried should lower to a pipe expression");
    let ExprKind::Pipe(pipe) = &lowered.module().exprs()[body].kind else {
        panic!("expected retried to lower as a pipe expression");
    };
    let recurrence = pipe
        .recurrence_suffix()
        .expect("lowered pipe should satisfy the structural recurrence invariant")
        .expect("retried should include a recurrence suffix");

    assert_eq!(
        recurrence.prefix_stage_count(),
        2,
        "prefix stages should stay separate from the recurrence suffix"
    );
    let prefix_kinds = recurrence
        .prefix_stages()
        .map(|stage| match &stage.kind {
            PipeStageKind::Transform { .. } => "transform",
            PipeStageKind::Tap { .. } => "tap",
            other => panic!("expected only non-recurrent prefix stages, found {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(prefix_kinds, vec!["transform", "tap"]);
    match &lowered.module().exprs()[recurrence.start_expr()].kind {
        ExprKind::Name(reference) => assert_eq!(path_text(&reference.path), "start"),
        other => panic!("expected recurrence start expression to stay a name, found {other:?}"),
    }
    assert_eq!(recurrence.step_count(), 2);
    let step_names = recurrence
        .step_exprs()
        .map(|expr_id| match &lowered.module().exprs()[expr_id].kind {
            ExprKind::Name(reference) => path_text(&reference.path),
            other => {
                panic!("expected recurrence step expression to stay a name, found {other:?}")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(step_names, vec!["step".to_owned(), "step".to_owned()]);
}

#[test]
fn allows_recurrence_guards_before_steps() {
    let lowered = lower_text(
        "recurrence-guard-view.aivi",
        r#"domain Duration over Int = {
    suffix sec : Int = value => Duration value
}
type Cursor = { hasNext: Bool }
fun keep:Cursor = cursor:Cursor => cursor
value seed:Cursor = { hasNext: True }
@recur.timer 1sec
signal cursor : Signal Cursor =
 seed
  @|> keep
  ?|> .hasNext
  <|@ keep
"#,
    );
    assert!(
        !lowered.has_errors(),
        "recurrence guards should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let cursor = find_signal(lowered.module(), "cursor");
    let body = cursor
        .body
        .expect("cursor should lower to a pipe expression");
    let ExprKind::Pipe(pipe) = &lowered.module().exprs()[body].kind else {
        panic!("expected cursor to lower as a pipe expression");
    };
    let recurrence = pipe
        .recurrence_suffix()
        .expect("lowered pipe should satisfy the structural recurrence invariant")
        .expect("cursor should include a recurrence suffix");

    assert_eq!(recurrence.guard_stage_count(), 1);
    assert_eq!(recurrence.step_count(), 1);
}

#[test]
fn allows_fanout_filters_before_join() {
    let lowered = lower_text(
        "fanout-filter-before-join.aivi",
        r#"type User = { email: Text }
fun keepText:Bool = email:Text => True
fun joinEmails:Text = items:List Text => "joined"
value users:List User = [{ email: "ada@example.com" }]
value joinedEmails:Text =
 users
  *|> .email
  ?|> keepText
  <|* joinEmails
"#,
    );
    assert!(
        !lowered.has_errors(),
        "fan-out filters before `<|*` should lower cleanly: {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn normalizes_standalone_function_signature_lines_into_parameter_annotations() {
    let lowered = lower_text(
        "standalone-function-signature.aivi",
        "type List Text -> Text\n\
             func joinEmails = items=> \"joined\"\n",
    );
    assert!(
        !lowered.has_errors(),
        "standalone function signature lines should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let function = match find_named_item(lowered.module(), "joinEmails") {
        Item::Function(item) => item,
        other => panic!("expected `joinEmails` to lower as a function item, found {other:?}"),
    };
    assert!(
        function.context.is_empty(),
        "plain parameter types should not remain in the class-constraint context"
    );
    let parameter_annotation = function.parameters[0]
        .annotation
        .expect("parameter annotation should be reconstructed from the signature line");
    let result_annotation = function
        .annotation
        .expect("result annotation should remain attached to the function");

    match &lowered.module().types()[parameter_annotation].kind {
        TypeKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            match &lowered.module().types()[*callee].kind {
                TypeKind::Name(reference) => assert!(
                    matches!(
                        reference.resolution.as_ref(),
                        ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List))
                    ),
                    "expected builtin `List` callee resolution, found {:?}",
                    reference.resolution.as_ref()
                ),
                other => {
                    panic!("expected `List` callee in parameter annotation, found {other:?}")
                }
            }
            let argument = arguments
                .iter()
                .next()
                .copied()
                .expect("list annotation should keep its element type");
            match &lowered.module().types()[argument].kind {
                TypeKind::Name(reference) => assert!(
                    matches!(
                        reference.resolution.as_ref(),
                        ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
                    ),
                    "expected builtin `Text` element resolution, found {:?}",
                    reference.resolution.as_ref()
                ),
                other => panic!("expected `Text` list element annotation, found {other:?}"),
            }
        }
        other => panic!("expected `List Text` parameter annotation, found {other:?}"),
    }

    match &lowered.module().types()[result_annotation].kind {
        TypeKind::Name(reference) => assert!(
            matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
            ),
            "expected builtin `Text` result resolution, found {:?}",
            reference.resolution.as_ref()
        ),
        other => panic!("expected `Text` result annotation, found {other:?}"),
    }
}

#[test]
fn preserves_real_class_constraints_while_normalizing_function_signatures() {
    let lowered = lower_text(
        "standalone-function-signature-with-constraint.aivi",
        "type Setoid Text => Text -> Bool\n\
             func same = text=> True\n",
    );
    assert!(
        !lowered.has_errors(),
        "class constraints should remain intact while normalizing function signatures: {:?}",
        lowered.diagnostics()
    );

    let function = match find_named_item(lowered.module(), "same") {
        Item::Function(item) => item,
        other => panic!("expected `same` to lower as a function item, found {other:?}"),
    };
    assert_eq!(
        function.context.len(),
        1,
        "the Setoid constraint should stay in the function context"
    );
    let constraint = function.context[0];
    match &lowered.module().types()[constraint].kind {
        TypeKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            match &lowered.module().types()[*callee].kind {
                TypeKind::Name(reference) => match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                        assert!(matches!(lowered.module().items()[*item_id], Item::Class(_)));
                    }
                    other => panic!("expected class resolution for `Setoid`, found {other:?}"),
                },
                other => panic!("expected `Setoid` callee in constraint, found {other:?}"),
            }
        }
        other => panic!("expected `Setoid Text` class constraint, found {other:?}"),
    }
    assert!(
        function.parameters[0].annotation.is_some(),
        "the function parameter should still receive its normalized annotation"
    );
    assert!(
        function.annotation.is_some(),
        "the result annotation should still be present after normalization"
    );
}

#[test]
fn normalizes_legacy_constraint_arrow_for_applied_parameter_types() {
    let lowered = lower_text(
        "standalone-function-signature-legacy-constraint-arrow.aivi",
        "type List Text => Text\n\
             func joinEmails = items=> \"joined\"\n",
    );
    assert!(
        !lowered.has_errors(),
        "legacy formatter output should still normalize into parameter annotations: {:?}",
        lowered.diagnostics()
    );

    let function = match find_named_item(lowered.module(), "joinEmails") {
        Item::Function(item) => item,
        other => panic!("expected `joinEmails` to lower as a function item, found {other:?}"),
    };
    assert!(
        function.context.is_empty(),
        "legacy `=>` output should not leave applied parameter types in the class-constraint context"
    );
    assert!(
        function.parameters[0].annotation.is_some(),
        "legacy formatter output should still reconstruct the parameter annotation"
    );
    assert!(
        function.annotation.is_some(),
        "legacy formatter output should keep the result annotation"
    );
}

#[test]
fn snake_demo_legacy_signature_lines_keep_all_parameter_annotations() {
    // Inline source that tests the "legacy signature lines" format where function
    // parameters carry type annotations. This format must survive HIR lowering.
    let lowered = lower_text(
        "demos/snake.aivi",
        r#"
fun bodyOrFoodGlyph:Text = isHead:Bool isBody:Bool isFood:Bool =>
    isHead | isBody | isFood | "."

fun cellGlyph:Text = row:Int col:Int =>
    bodyOrFoodGlyph False False False

fun rowTextStep:Text = row:Int acc:Text col:Int =>
    "{acc}{cellGlyph row col}"

fun rowText:Text = row:Int =>
    ""

fun boardTextStep:Text = acc:Text row:Int =>
    "{acc}{rowText row}"
"#,
    );
    assert!(
        !lowered.has_errors(),
        "snake demo should still lower cleanly after signature normalization: {:?}",
        lowered.diagnostics()
    );

    for name in [
        "bodyOrFoodGlyph",
        "cellGlyph",
        "rowTextStep",
        "rowText",
        "boardTextStep",
    ] {
        let function = match find_named_item(lowered.module(), name) {
            Item::Function(item) => item,
            other => panic!("expected `{name}` to lower as a function item, found {other:?}"),
        };
        assert!(
            function
                .parameters
                .iter()
                .all(|parameter| parameter.annotation.is_some()),
            "expected `{name}` to keep all parameter annotations, found {:?}",
            function
                .parameters
                .iter()
                .map(|parameter| parameter.annotation.is_some())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn lowers_trailing_clusters_with_implicit_tuple_finalizers() {
    let lowered = lower_fixture("milestone-1/valid/pipes/applicative_clusters.aivi");
    assert!(
        !lowered.has_errors(),
        "cluster fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let tupled_names = find_signal(lowered.module(), "tupledNames");
    let body = tupled_names
        .body
        .expect("tupledNames signal should have a lowered cluster body");
    let cluster_id = match &lowered.module().exprs()[body].kind {
        ExprKind::Cluster(cluster) => *cluster,
        other => panic!("expected cluster expression, found {other:?}"),
    };
    assert!(
        matches!(
            lowered.module().clusters()[cluster_id].finalizer,
            ClusterFinalizer::ImplicitTuple
        ),
        "pipe-end clusters should lower with an implicit tuple finalizer"
    );
}

#[test]
fn bundle_imports_do_not_hijack_builtin_option_resolution() {
    let lowered = lower_fixture("milestone-1/valid/records/record_shorthand_and_elision.aivi");
    assert!(
        !lowered.has_errors(),
        "record shorthand fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    assert!(
        lowered
            .module()
            .imports()
            .iter()
            .any(|(_, import)| import.imported_name.text() == "Option"),
        "fixture should preserve the explicit Option bundle import"
    );
    assert!(
        lowered
            .module()
            .imports()
            .iter()
            .any(|(_, import)| matches!(
                import.metadata,
                ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
            )),
        "fixture should preserve builtin Option bundle metadata"
    );

    let option_refs = lowered
        .module()
        .types()
        .iter()
        .filter_map(|(_, ty)| match &ty.kind {
            TypeKind::Name(reference) if reference.path.segments().first().text() == "Option" => {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !option_refs.is_empty(),
        "expected Option references in the lowered HIR"
    );
    assert!(
        option_refs.iter().all(|reference| matches!(
            reference.resolution,
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option))
        )),
        "Option type references should resolve to the builtin even when a bundle import exists: {option_refs:?}"
    );
}

#[test]
fn use_member_imports_preserve_compiler_known_metadata() {
    let lowered = lower_fixture("milestone-2/valid/use-member-imports/main.aivi");
    assert!(
        !lowered.has_errors(),
        "use-member-imports fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let text_imports = lowered
        .module()
        .imports()
        .iter()
        .filter_map(
            |(_, import)| match (import.local_name.text(), &import.metadata) {
                ("http" | "socket", ImportBindingMetadata::Value { ty }) => Some(ty),
                _ => None,
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(
        text_imports.len(),
        2,
        "expected http/socket imports to carry compiler-known value metadata, got {:?}",
        lowered.module().imports().iter().collect::<Vec<_>>()
    );
    assert!(
        text_imports
            .iter()
            .all(|ty| matches!(ty, ImportValueType::Primitive(BuiltinType::Text))),
        "expected http/socket imports to lower as Text-valued bindings, got {text_imports:?}"
    );

    let request_import = lowered.module().imports().iter().find_map(|(_, import)| {
        match (import.local_name.text(), &import.metadata) {
            ("Request", ImportBindingMetadata::TypeConstructor { kind, .. }) => Some(kind),
            _ => None,
        }
    });
    assert_eq!(
        request_import,
        Some(&Kind::constructor(1)),
        "expected Request import to preserve unary constructor kind metadata"
    );

    let channel_import = lowered.module().imports().iter().find_map(|(_, import)| {
        match (import.local_name.text(), &import.metadata) {
            ("Channel", ImportBindingMetadata::TypeConstructor { kind, .. }) => Some(kind),
            _ => None,
        }
    });
    assert_eq!(
        channel_import,
        Some(&Kind::constructor(2)),
        "expected Channel import to preserve binary constructor kind metadata"
    );

    let imported_type_refs = lowered
        .module()
        .types()
        .iter()
        .filter_map(|(_, ty)| match &ty.kind {
            TypeKind::Name(reference)
                if matches!(
                    reference.path.segments().first().text(),
                    "Request" | "Channel"
                ) =>
            {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        imported_type_refs.len(),
        2,
        "expected Request/Channel references in lowered HIR"
    );
    assert!(
        imported_type_refs.iter().all(|reference| matches!(
            reference.resolution,
            ResolutionState::Resolved(TypeResolution::Import(_))
        )),
        "imported type references should resolve through import bindings: {imported_type_refs:?}"
    );
}

#[test]
fn use_member_import_aliases_preserve_local_names_and_metadata() {
    let lowered = lower_fixture("milestone-2/valid/use-member-import-aliases/main.aivi");
    assert!(
        !lowered.has_errors(),
        "use-member-import-aliases fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "use-member-import-aliases fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let primary_http = lowered.module().imports().iter().find(|(_, import)| {
        import.imported_name.text() == "http" && import.local_name.text() == "primaryHttp"
    });
    assert!(
        matches!(
            primary_http.map(|(_, import)| &import.metadata),
            Some(ImportBindingMetadata::Value {
                ty: ImportValueType::Primitive(BuiltinType::Text)
            })
        ),
        "expected aliased http import to preserve Text metadata"
    );

    let aliased_request = lowered.module().imports().iter().find(|(_, import)| {
        import.imported_name.text() == "Request" && import.local_name.text() == "HttpRequest"
    });
    assert!(
        matches!(
            aliased_request.map(|(_, import)| &import.metadata),
            Some(ImportBindingMetadata::TypeConstructor { kind, .. })
                if kind == &Kind::constructor(1)
        ),
        "expected aliased Request import to preserve constructor kind metadata"
    );

    let imported_type_refs = lowered
        .module()
        .types()
        .iter()
        .filter_map(|(_, ty)| match &ty.kind {
            TypeKind::Name(reference)
                if matches!(
                    reference.path.segments().first().text(),
                    "HttpRequest" | "NetworkChannel"
                ) =>
            {
                Some(reference)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        imported_type_refs.len(),
        2,
        "expected aliased imported type references in lowered HIR"
    );
    assert!(
        imported_type_refs.iter().all(|reference| matches!(
            reference.resolution,
            ResolutionState::Resolved(TypeResolution::Import(_))
        )),
        "aliased imported type references should still resolve through import bindings: {imported_type_refs:?}"
    );
}

#[test]
fn use_db_imports_preserve_intrinsic_metadata_for_builder_helpers() {
    let lowered = lower_text(
        "db-builder-helper-imports.aivi",
        "use aivi.db (paramInt, statement)\n",
    );
    assert!(
        !lowered.has_errors(),
        "db builder import fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let imported = lowered
        .module()
        .imports()
        .iter()
        .map(|(_, import)| {
            (
                import.imported_name.text().to_owned(),
                import.metadata.clone(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(
        imported.get("paramInt"),
        Some(&ImportBindingMetadata::IntrinsicValue {
            value: IntrinsicValue::DbParamInt,
            ty: super::arrow_import_type(
                super::primitive_import_type(BuiltinType::Int),
                super::db_param_import_type(),
            ),
        })
    );
    assert_eq!(
        imported.get("statement"),
        Some(&ImportBindingMetadata::IntrinsicValue {
            value: IntrinsicValue::DbStatement,
            ty: super::arrow_import_type(
                super::primitive_import_type(BuiltinType::Text),
                super::arrow_import_type(
                    super::list_import_type(super::db_param_import_type()),
                    super::db_statement_import_type(),
                ),
            ),
        })
    );
}

#[test]
fn lowers_domains_with_carriers_parameters_and_members() {
    let lowered = lower_fixture("milestone-2/valid/domain-declarations/main.aivi");
    assert!(
        !lowered.has_errors(),
        "domain fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "domain fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let path = match find_named_item(lowered.module(), "Path") {
        Item::Domain(item) => item,
        other => panic!("expected `Path` to lower as a domain item, found {other:?}"),
    };
    assert!(matches!(
        lowered.module().types()[path.carrier].kind,
        TypeKind::Name(ref reference)
            if matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
            )
    ));
    assert_eq!(path.members.len(), 2);
    assert!(matches!(path.members[0].kind, DomainMemberKind::Literal));
    assert_eq!(path.members[0].name.text(), "root");
    assert!(matches!(path.members[1].kind, DomainMemberKind::Operator));
    assert_eq!(path.members[1].name.text(), "/");

    let non_empty = match find_named_item(lowered.module(), "NonEmpty") {
        Item::Domain(item) => item,
        other => panic!("expected `NonEmpty` to lower as a domain item, found {other:?}"),
    };
    assert_eq!(non_empty.parameters.len(), 1);
}

#[test]
fn lowers_instances_with_same_module_class_resolution_and_local_parameters() {
    let lowered = lower_fixture("milestone-2/valid/instance-declarations/main.aivi");
    assert!(
        !lowered.has_errors(),
        "instance fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "instance fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let instance = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Instance(item) => Some(item),
            _ => None,
        })
        .expect("fixture should lower one instance item");
    assert_eq!(instance.arguments.len(), 1);
    assert!(matches!(
        instance.class.resolution,
        ResolutionState::Resolved(TypeResolution::Item(class_item))
            if matches!(&lowered.module().items()[class_item], Item::Class(class) if class.name.text() == "Eq")
    ));
    assert_eq!(instance.members.len(), 1);
    assert_eq!(instance.members[0].parameters.len(), 2);

    let ExprKind::Apply { arguments, .. } =
        &lowered.module().exprs()[instance.members[0].body].kind
    else {
        panic!("expected instance body to lower as an application");
    };
    let argument_kinds = arguments
        .iter()
        .map(|argument| match &lowered.module().exprs()[*argument].kind {
            ExprKind::Name(reference) => reference.resolution.clone(),
            other => panic!("expected local instance member arguments, found {other:?}"),
        })
        .collect::<Vec<_>>();
    assert!(argument_kinds.iter().all(|resolution| matches!(
        resolution,
        ResolutionState::Resolved(TermResolution::Local(_))
    )));
}

#[test]
fn rejects_duplicate_instances_during_validation() {
    let lowered = lower_fixture("milestone-2/invalid/duplicate-instance/main.aivi");
    assert!(
        !lowered.has_errors(),
        "duplicate-instance fixture should lower cleanly before validation: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-instance"))),
        "expected duplicate-instance validation diagnostic, got {:?}",
        report.diagnostics()
    );
}

#[test]
fn preserves_domain_member_ambiguity_for_contextual_resolution() {
    let lowered = lower_fixture("milestone-2/valid/domain-member-resolution/main.aivi");
    assert!(
        !lowered.has_errors(),
        "domain-member-resolution fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "domain-member-resolution fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let delay = match find_named_item(lowered.module(), "delay") {
        Item::Value(item) => item,
        other => panic!("expected `delay` to lower as a value item, found {other:?}"),
    };
    let ExprKind::Apply { callee, .. } = &lowered.module().exprs()[delay.body].kind else {
        panic!("expected `delay` body to lower as an application");
    };
    let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
        panic!("expected `delay` callee to stay a name");
    };
    assert!(
        matches!(
            reference.resolution,
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(ref candidates))
                if candidates.len() == 2
        ),
        "expected `make` to preserve both domain candidates for later contextual resolution, found {:?}",
        reference.resolution
    );
}

#[test]
fn lowers_authored_domain_member_bindings_into_hir_members() {
    let lowered = lower_text(
        "domain-authored-members.aivi",
        r#"
type Builder = Int -> Duration

domain Duration over Int = {
    make : Builder
    make raw = raw
    unwrap : Duration -> Int
    unwrap duration = duration
}"#,
    );
    assert!(
        !lowered.has_errors(),
        "authored domain members should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let domain = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Domain(item) => Some(item),
            _ => None,
        })
        .expect("fixture should lower one domain item");
    assert_eq!(domain.members.len(), 2);
    assert_eq!(domain.members[0].parameters.len(), 1);
    assert!(domain.members[0].body.is_some());
    assert_eq!(domain.members[1].parameters.len(), 1);
    assert!(domain.members[1].body.is_some());
}

#[test]
fn lowers_type_companion_members_into_synthetic_functions() {
    let lowered = lower_text(
        "type-companions.aivi",
        r#"
type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human
}

value next : Player = opponent Human
"#,
    );
    assert!(
        !lowered.has_errors(),
        "type companions should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "type companions should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let companion = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Function(item) if item.name.text() == "opponent" => Some(item),
            _ => None,
        })
        .expect("fixture should lower one synthetic companion function");
    assert_eq!(companion.parameters.len(), 1);
    let parameter_annotation = companion.parameters[0]
        .annotation
        .expect("explicit companion receiver type should split onto the parameter");
    let TypeKind::Name(parameter_ref) = &lowered.module().types()[parameter_annotation].kind else {
        panic!("companion parameter type should lower to Player");
    };
    let result_annotation = companion
        .annotation
        .expect("explicit companion result type should remain on the function");
    let TypeKind::Name(result_ref) = &lowered.module().types()[result_annotation].kind else {
        panic!("companion result type should lower to Player");
    };
    assert_eq!(parameter_ref.path.to_string(), "Player");
    assert_eq!(result_ref.path.to_string(), "Player");
}

#[test]
fn lowers_inline_annotated_type_companion_members_into_synthetic_functions() {
    let lowered = lower_text(
        "inline-type-companions.aivi",
        r#"
type Player = {
    | Human
    | Computer

    opponent: Player -> Player = .
     ||> Human    -> Computer
     ||> Computer -> Human
}

value next : Player = opponent Human
"#,
    );
    assert!(
        !lowered.has_errors(),
        "inline type companions should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "inline type companions should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let companion = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Function(item) if item.name.text() == "opponent" => Some(item),
            _ => None,
        })
        .expect("fixture should lower one synthetic companion function");
    assert_eq!(companion.parameters.len(), 1);
    let parameter_annotation = companion.parameters[0]
        .annotation
        .expect("inline companion receiver type should split onto the parameter");
    let TypeKind::Name(parameter_ref) = &lowered.module().types()[parameter_annotation].kind else {
        panic!("companion parameter type should lower to Player");
    };
    let result_annotation = companion
        .annotation
        .expect("inline companion result type should remain on the function");
    let TypeKind::Name(result_ref) = &lowered.module().types()[result_annotation].kind else {
        panic!("companion result type should lower to Player");
    };
    assert_eq!(parameter_ref.path.to_string(), "Player");
    assert_eq!(result_ref.path.to_string(), "Player");
}

#[test]
fn type_companion_members_capture_owner_type_parameters() {
    let lowered = lower_text(
        "generic-type-companions.aivi",
        r#"
type Box A = {
    | Box A

    type Box A -> A
    unbox = .
     ||> Box value -> value
}

value current : Int = unbox (Box 1)
"#,
    );
    assert!(
        !lowered.has_errors(),
        "generic type companions should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "generic type companions should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let companion = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Function(item) if item.name.text() == "unbox" => Some(item),
            _ => None,
        })
        .expect("fixture should lower one generic companion function");
    assert_eq!(companion.parameters.len(), 1);
    assert!(
        !companion.type_parameters.is_empty(),
        "generic companion should retain the owner type parameters"
    );
}

#[test]
fn resolves_suffixed_integers_to_domain_literal_declarations() {
    let lowered = lower_fixture("milestone-2/valid/domain-literal-suffixes/main.aivi");
    assert!(
        !lowered.has_errors(),
        "domain literal suffix fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "domain literal suffix fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let duration_domain_id = lowered
        .module()
        .root_items()
        .iter()
        .copied()
        .find(|item_id| {
            matches!(
                &lowered.module().items()[*item_id],
                Item::Domain(item) if item.name.text() == "Duration"
            )
        })
        .expect("fixture should contain Duration domain");

    let delay_body = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Value(item) if item.name.text() == "delay" => Some(item.body),
            _ => None,
        })
        .expect("fixture should contain delay value");

    match &lowered.module().exprs()[delay_body].kind {
        ExprKind::SuffixedInteger(literal) => {
            assert_eq!(&*literal.raw, "250");
            assert_eq!(literal.suffix.text(), "ms");
            assert_eq!(
                literal.resolution,
                ResolutionState::Resolved(LiteralSuffixResolution::DomainMember(
                    DomainMemberResolution {
                        domain: duration_domain_id,
                        member_index: 0,
                    }
                ))
            );
        }
        other => panic!("expected suffixed integer expression, found {other:?}"),
    }
}

#[test]
fn lowers_builtin_noninteger_literals_and_preserves_raw_spelling() {
    let lowered = lower_text(
        "builtin-noninteger-literals.aivi",
        "value pi:Float = 3.14\n\
             value amount:Decimal = 19.25d\n\
             value whole:Decimal = 19d\n\
             value count:BigInt = 123n\n",
    );
    assert!(
        !lowered.has_errors(),
        "builtin noninteger literal source should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let Item::Value(pi) = find_named_item(lowered.module(), "pi") else {
        panic!("expected `pi` to be a value item");
    };
    assert!(matches!(
        &lowered.module().exprs()[pi.body].kind,
        ExprKind::Float(literal) if &*literal.raw == "3.14"
    ));

    let Item::Value(amount) = find_named_item(lowered.module(), "amount") else {
        panic!("expected `amount` to be a value item");
    };
    assert!(matches!(
        &lowered.module().exprs()[amount.body].kind,
        ExprKind::Decimal(literal) if &*literal.raw == "19.25d"
    ));

    let Item::Value(whole) = find_named_item(lowered.module(), "whole") else {
        panic!("expected `whole` to be a value item");
    };
    assert!(matches!(
        &lowered.module().exprs()[whole.body].kind,
        ExprKind::Decimal(literal) if &*literal.raw == "19d"
    ));

    let Item::Value(count) = find_named_item(lowered.module(), "count") else {
        panic!("expected `count` to be a value item");
    };
    assert!(matches!(
        &lowered.module().exprs()[count.body].kind,
        ExprKind::BigInt(literal) if &*literal.raw == "123n"
    ));
}

#[test]
fn lowers_map_and_set_literals() {
    let lowered = lower_text(
        "map-set-literals.aivi",
        "value headers = Map { \"x\": 1, \"y\": 2 }\nvalue tags = Set [\"a\", \"b\"]\n",
    );
    assert!(
        !lowered.has_errors(),
        "map/set literal source should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let headers_body = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Value(item) if item.name.text() == "headers" => Some(item.body),
            _ => None,
        })
        .expect("fixture should contain headers value");
    match &lowered.module().exprs()[headers_body].kind {
        ExprKind::Map(map) => {
            assert_eq!(map.entries.len(), 2);
            assert!(matches!(
                lowered.module().exprs()[map.entries[0].key].kind,
                ExprKind::Text(_)
            ));
            assert!(matches!(
                lowered.module().exprs()[map.entries[0].value].kind,
                ExprKind::Integer(_)
            ));
        }
        other => panic!("expected map literal expression, found {other:?}"),
    }

    let tags_body = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Value(item) if item.name.text() == "tags" => Some(item.body),
            _ => None,
        })
        .expect("fixture should contain tags value");
    match &lowered.module().exprs()[tags_body].kind {
        ExprKind::Set(elements) => {
            assert_eq!(elements.len(), 2);
            assert!(matches!(
                lowered.module().exprs()[elements[0]].kind,
                ExprKind::Text(_)
            ));
        }
        other => panic!("expected set literal expression, found {other:?}"),
    }
}

#[test]
fn duplicate_map_keys_report_hir_diagnostics() {
    let lowered = lower_text(
        "duplicate-map-key.aivi",
        "value headers = Map { \"Authorization\": \"a\", \"Authorization\": \"b\" }\n",
    );
    assert!(
        lowered.has_errors(),
        "duplicate map key should fail lowering"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-map-key"))),
        "expected duplicate-map-key diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn duplicate_record_fields_report_hir_diagnostics() {
    let lowered = lower_text(
        "duplicate-record-field.aivi",
        "type User = { name: Text }\nvalue user:User = { name: \"Ada\", name: \"Grace\" }\n",
    );
    assert!(
        lowered.has_errors(),
        "duplicate record fields should fail lowering"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
        "expected duplicate-record-field diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn duplicate_record_type_fields_report_hir_diagnostics() {
    let lowered = lower_text(
        "duplicate-record-type-field.aivi",
        "type User = { name: Text, age: Int, name: Bool }\n",
    );
    assert!(
        lowered.has_errors(),
        "duplicate record type fields should fail lowering"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
        "expected duplicate-record-field diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn duplicate_record_pattern_fields_report_hir_diagnostics() {
    let lowered = lower_text(
        "duplicate-record-pattern-field.aivi",
        "type User = { name: Text }\nfun extract:Text = user:User =>\n    user\n     ||> { name, name } -> name\n",
    );
    assert!(
        lowered.has_errors(),
        "duplicate record pattern fields should fail lowering"
    );
    assert!(
        lowered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
        "expected duplicate-record-field diagnostic, got {:?}",
        lowered.diagnostics()
    );
}

#[test]
fn duplicate_set_elements_warn_and_canonicalize() {
    let lowered = lower_text(
        "duplicate-set-element.aivi",
        "value tags = Set [\"news\", \"featured\", \"news\"]\n",
    );
    assert!(
        !lowered.has_errors(),
        "duplicate set elements should canonicalize without a lowering error: {:?}",
        lowered.diagnostics()
    );
    assert!(lowered.diagnostics().iter().any(|diagnostic| {
        diagnostic.severity == Severity::Warning
            && diagnostic.code == Some(super::code("duplicate-set-element"))
    }));
    let tags_body = lowered
        .module()
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Value(item) if item.name.text() == "tags" => Some(item.body),
            _ => None,
        })
        .expect("fixture should contain tags value");
    match &lowered.module().exprs()[tags_body].kind {
        ExprKind::Set(elements) => {
            assert_eq!(elements.len(), 2, "set literal should be canonicalized");
            assert!(matches!(
                lowered.module().exprs()[elements[0]].kind,
                ExprKind::Text(_)
            ));
            assert!(matches!(
                lowered.module().exprs()[elements[1]].kind,
                ExprKind::Text(_)
            ));
        }
        other => panic!("expected set literal expression, found {other:?}"),
    }
}

#[test]
fn exports_can_target_constructors_through_parent_type_items() {
    let lowered = lower_text(
        "constructor-export.aivi",
        "type Status = Idle | Busy\nexport Idle\n",
    );
    assert!(
        !lowered.has_errors(),
        "constructor export source should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "constructor export should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let export = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Export(item) => Some(item),
            _ => None,
        })
        .expect("constructor-export source should contain one export item");

    let resolved = match export.resolution {
        ResolutionState::Resolved(item) => item,
        ResolutionState::Unresolved => panic!("constructor export should resolve"),
    };
    let ExportResolution::Item(resolved) = resolved else {
        panic!("constructor export should resolve to the parent type item");
    };
    match &lowered.module().items()[resolved] {
        Item::Type(item) => assert_eq!(item.name.text(), "Status"),
        other => {
            panic!("constructor export should resolve to the parent type item, found {other:?}")
        }
    }
}

#[test]
fn grouped_exports_lower_to_individual_resolved_hir_items() {
    let lowered = lower_text(
        "grouped-export.aivi",
        "type Status = Idle | Busy\nvalue main = Idle\nexport (Idle, main)\n",
    );
    assert!(
        !lowered.has_errors(),
        "grouped export source should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "grouped export source should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let exports = lowered
        .module()
        .root_items()
        .iter()
        .filter_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Export(item) => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        exports.len(),
        2,
        "grouped export should lower to two HIR export items"
    );
    assert_eq!(
        exports
            .iter()
            .map(|export| export.target.segments().first().text())
            .collect::<Vec<_>>(),
        vec!["Idle", "main"]
    );

    let exported_names = crate::exports::exports(lowered.module());
    assert!(exported_names.find("main").is_some());
    assert!(exported_names.find("Idle").is_some());
    assert!(exported_names.find("Status").is_none());
}

#[test]
fn exports_support_builtin_and_ambient_root_surface_targets() {
    let lowered = lower_text(
        "builtin-export.aivi",
        "export (Int, Option, Some, Eq, Foldable)\n",
    );
    assert!(
        !lowered.has_errors(),
        "builtin export source should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "builtin export source should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let exported_names = exports(lowered.module());
    assert_eq!(
        exported_names
            .find("Int")
            .map(|exported| &exported.metadata),
        Some(&ImportBindingMetadata::BuiltinType(BuiltinType::Int))
    );
    assert_eq!(
        exported_names
            .find("Option")
            .map(|exported| &exported.metadata),
        Some(&ImportBindingMetadata::BuiltinType(BuiltinType::Option))
    );
    assert_eq!(
        exported_names
            .find("Some")
            .map(|exported| &exported.metadata),
        Some(&ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Some))
    );
    assert_eq!(
        exported_names.find("Eq").map(|exported| &exported.metadata),
        Some(&ImportBindingMetadata::AmbientType)
    );
    assert_eq!(
        exported_names
            .find("Foldable")
            .map(|exported| &exported.metadata),
        Some(&ImportBindingMetadata::AmbientType)
    );
}

#[test]
fn local_module_definitions_shadow_builtins() {
    let lowered = lower_text(
        "builtin-shadowing.aivi",
        concat!(
            "value True = 0\n",
            "value chosen = True\n",
            "type Option = Option Int\n",
            "value wrapped:Option = Option 1\n",
        ),
    );
    assert!(
        !lowered.has_errors(),
        "builtin shadowing source should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "builtin shadowing source should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let chosen = match find_named_item(lowered.module(), "chosen") {
        Item::Value(item) => item,
        other => panic!("expected chosen to be a value item, found {other:?}"),
    };
    let chosen_resolution = match &lowered.module().exprs()[chosen.body].kind {
        ExprKind::Name(reference) => &reference.resolution,
        other => panic!("expected chosen body to be a name, found {other:?}"),
    };
    assert!(
        matches!(
            chosen_resolution,
            ResolutionState::Resolved(TermResolution::Item(_))
        ),
        "local term definitions should shadow builtin terms: {chosen_resolution:?}"
    );

    let wrapped = match find_named_item(lowered.module(), "wrapped") {
        Item::Value(item) => item,
        other => panic!("expected wrapped to be a value item, found {other:?}"),
    };
    let annotation = wrapped
        .annotation
        .expect("wrapped should preserve its type annotation");
    let annotation_resolution = match &lowered.module().types()[annotation].kind {
        TypeKind::Name(reference) => &reference.resolution,
        other => panic!("expected wrapped annotation to be a name, found {other:?}"),
    };
    assert!(
        matches!(
            annotation_resolution,
            ResolutionState::Resolved(TypeResolution::Item(_))
        ),
        "local type definitions should shadow builtin types: {annotation_resolution:?}"
    );
}

#[test]
fn local_domain_literal_suffixes_shadow_ambient_stdlib_suffixes() {
    let lowered = crate::test_support::lower_text_with_stdlib(
        "local-domain-suffix-shadowing.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}

domain Retry over Int = {
    suffix times : Int = value => Retry value
}

value timeout : Duration = 5sec
value retries : Retry = 3times
"#,
    );
    assert!(
        !lowered.has_errors(),
        "local domain suffix shadowing should lower cleanly with stdlib hoists: {:?}",
        lowered.diagnostics()
    );

    let duration_domain_id = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Domain(item) if item.name.text() == "Duration" => Some(*item_id),
            _ => None,
        })
        .expect("fixture should define a local Duration domain");
    let retry_domain_id = lowered
        .module()
        .root_items()
        .iter()
        .find_map(|item_id| match &lowered.module().items()[*item_id] {
            Item::Domain(item) if item.name.text() == "Retry" => Some(*item_id),
            _ => None,
        })
        .expect("fixture should define a local Retry domain");

    let timeout = match find_named_item(lowered.module(), "timeout") {
        Item::Value(item) => item,
        other => panic!("expected timeout to be a value item, found {other:?}"),
    };
    match &lowered.module().exprs()[timeout.body].kind {
        ExprKind::SuffixedInteger(literal) => {
            assert_eq!(literal.suffix.text(), "sec");
            assert_eq!(
                literal.resolution,
                ResolutionState::Resolved(LiteralSuffixResolution::DomainMember(
                    DomainMemberResolution {
                        domain: duration_domain_id,
                        member_index: 0,
                    }
                ))
            );
        }
        other => panic!("expected timeout body to be a suffixed integer, found {other:?}"),
    }

    let retries = match find_named_item(lowered.module(), "retries") {
        Item::Value(item) => item,
        other => panic!("expected retries to be a value item, found {other:?}"),
    };
    match &lowered.module().exprs()[retries.body].kind {
        ExprKind::SuffixedInteger(literal) => {
            assert_eq!(literal.suffix.text(), "times");
            assert_eq!(
                literal.resolution,
                ResolutionState::Resolved(LiteralSuffixResolution::DomainMember(
                    DomainMemberResolution {
                        domain: retry_domain_id,
                        member_index: 0,
                    }
                ))
            );
        }
        other => panic!("expected retries body to be a suffixed integer, found {other:?}"),
    }
}

#[test]
fn lowers_result_blocks_into_nested_result_case_pipes() {
    let lowered = lower_fixture("milestone-2/valid/result-block/main.aivi");
    assert!(
        !lowered.has_errors(),
        "result block fixture should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    let report = lowered
        .module()
        .validate(ValidationMode::RequireResolvedNames);
    assert!(
        report.is_ok(),
        "result block fixture should validate as resolved HIR: {:?}",
        report.diagnostics()
    );

    let combined = match find_named_item(lowered.module(), "combined") {
        Item::Value(item) => item,
        other => panic!("expected combined to be a value item, found {other:?}"),
    };
    let ExprKind::Pipe(outer_pipe) = &lowered.module().exprs()[combined.body].kind else {
        panic!("expected combined body to lower into a pipe");
    };
    let outer_stages = outer_pipe.stages.iter().collect::<Vec<_>>();
    assert_eq!(
        outer_stages.len(),
        2,
        "each binding should lower into Ok/Err case arms"
    );
    assert!(matches!(outer_stages[0].kind, PipeStageKind::Case { .. }));
    assert!(matches!(outer_stages[1].kind, PipeStageKind::Case { .. }));

    let PipeStageKind::Case {
        body: inner_body, ..
    } = &outer_stages[0].kind
    else {
        panic!("expected first outer stage to be an Ok case arm");
    };
    let ExprKind::Pipe(inner_pipe) = &lowered.module().exprs()[*inner_body].kind else {
        panic!("expected Ok branch to continue with the nested result binding");
    };
    assert_eq!(inner_pipe.stages.iter().count(), 2);

    let rejected = match find_named_item(lowered.module(), "rejected") {
        Item::Value(item) => item,
        other => panic!("expected rejected to be a value item, found {other:?}"),
    };
    let ExprKind::Pipe(rejected_pipe) = &lowered.module().exprs()[rejected.body].kind else {
        panic!("expected rejected body to lower into a pipe");
    };
    let rejected_stages = rejected_pipe.stages.iter().collect::<Vec<_>>();
    // `rejected` has two bindings and no explicit tail; the outer Ok(left) branch continues
    // into the inner pipe for `right <- requirePositive 22`, whose Ok(right) branch
    // carries the implicit `Ok right` constructor application.
    let PipeStageKind::Case {
        body: outer_ok_body,
        ..
    } = &rejected_stages[0].kind
    else {
        panic!("expected rejected outer Ok branch");
    };
    let ExprKind::Pipe(inner_rejected_pipe) = &lowered.module().exprs()[*outer_ok_body].kind else {
        panic!("expected outer Ok branch to continue into inner pipe for the second binding");
    };
    let inner_rejected_stages = inner_rejected_pipe.stages.iter().collect::<Vec<_>>();
    let PipeStageKind::Case {
        body: implicit_tail,
        ..
    } = &inner_rejected_stages[0].kind
    else {
        panic!("expected inner Ok branch to be the implicit tail");
    };
    let ExprKind::Apply { .. } = &lowered.module().exprs()[*implicit_tail].kind else {
        panic!("implicit result tails should lower into an `Ok ...` constructor application");
    };
}

#[test]
fn normalizer_does_not_treat_constructor_type_as_class_constraint() {
    // Standalone type annotations starting with (List A) -> must be parsed as
    // function types, NOT as (List A) => ... constraints.
    // This was a bug: consume_constraint_separator accepted both => and ->.
    let lowered = lower_text(
        "constructor-type-not-constraint.aivi",
        "type (List A) -> (Option A) -> (List A)\n\
             func appendPrev = items prev => items\n",
    );
    assert!(
        !lowered.has_errors(),
        "function with (List A) -> parameter type should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let function = match find_named_item(lowered.module(), "appendPrev") {
        Item::Function(item) => item,
        other => panic!("expected function, got {other:?}"),
    };
    assert!(
        function.context.is_empty(),
        "no class constraints — (List A) is a constructor, not a class"
    );
    assert_eq!(
        function.parameters.len(),
        2,
        "function should have 2 parameters from the normalized annotation"
    );
    assert!(
        function.parameters[0].annotation.is_some(),
        "first parameter should receive a List A annotation"
    );
    assert!(
        function.parameters[1].annotation.is_some(),
        "second parameter should receive an Option A annotation"
    );
}

#[test]
fn ambient_matrix_at_row_has_correct_list_input_type() {
    // __aivi_matrix_atRow takes Option(List A) and returns Option A,
    // not Option A -> Int -> Option A (which was the old incorrect signature).
    let lowered = lower_text(
        "matrix-at-row-type.aivi",
        "use aivi.matrix (Matrix)\n\
             value x:Int = 1\n",
    );
    assert!(
        !lowered.has_errors(),
        "matrix import should lower cleanly: {:?}",
        lowered.diagnostics()
    );

    let function = lowered
        .module()
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Function(f) if f.name.text() == "__aivi_matrix_atRow" => Some(f),
            _ => None,
        })
        .expect("ambient prelude should contain __aivi_matrix_atRow");
    assert_eq!(
        function.parameters.len(),
        2,
        "atRow should have 2 parameters: rowOpt and x"
    );
}

#[test]
fn hoist_item_lowers_to_hir() {
    let lowered = lower_text("hoist-basic.aivi", "hoist\n");
    let hoist_item = lowered
        .module()
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Hoist(h) => Some(h.clone()),
            _ => None,
        })
        .expect("lowered module should contain a hoist item");
    assert!(hoist_item.kind_filters.is_empty());
    assert!(hoist_item.hiding.is_empty());
}

#[test]
fn hoist_item_lowers_kind_filters_correctly() {
    let lowered = lower_text("hoist-filters.aivi", "hoist (func, value)\n");
    let hoist_item = lowered
        .module()
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Hoist(h) => Some(h.clone()),
            _ => None,
        })
        .expect("lowered module should contain a hoist item");
    assert_eq!(hoist_item.kind_filters.len(), 2);
    assert!(matches!(hoist_item.kind_filters[0], HoistKindFilter::Func));
    assert!(matches!(hoist_item.kind_filters[1], HoistKindFilter::Value));
}

#[test]
fn hoist_item_lowers_hiding_list_correctly() {
    let lowered = lower_text("hoist-hiding.aivi", "hoist hiding (length, head)\n");
    let hoist_item = lowered
        .module()
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Hoist(h) => Some(h.clone()),
            _ => None,
        })
        .expect("lowered module should contain a hoist item");
    assert!(hoist_item.kind_filters.is_empty());
    assert_eq!(hoist_item.hiding.len(), 2);
    assert_eq!(hoist_item.hiding[0].text(), "length");
    assert_eq!(hoist_item.hiding[1].text(), "head");
}

#[test]
fn hoist_item_emits_diagnostic_for_unknown_kind_filter() {
    let lowered = lower_text("hoist-bad-filter.aivi", "hoist (funky)\n");
    assert!(
        lowered.has_errors(),
        "hoist with invalid kind filter should emit a diagnostic"
    );
    assert!(
        lowered.diagnostics().iter().any(|d| d
            .code
            .as_ref()
            .is_some_and(|c| c.name() == "unknown-hoist-kind-filter")),
        "expected unknown-hoist-kind-filter diagnostic, got {:?}",
        lowered.diagnostics()
    );
}
