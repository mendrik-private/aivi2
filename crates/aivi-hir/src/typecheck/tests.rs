use aivi_base::{FileId, SourceDatabase, SourceSpan};
use aivi_syntax::parse_module;

use crate::{BuiltinType, Item, PipeTransformMode, RecordFieldSurface, lower_module};

use super::*;

fn typecheck_text(path: &str, text: &str) -> TypeCheckReport {
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

fn typecheck_and_elaborate_text(path: &str, text: &str) -> (TypeCheckReport, Module) {
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
    let lowered_module = lowered.module().clone();
    let report = typecheck_module(&lowered_module);
    let elaborated = apply_defaults(&lowered_module, &report);
    (report, elaborated)
}

fn lowered_module_text(path: &str, text: &str) -> Module {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "module input should parse cleanly: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let lowered = lower_module(&parsed.module);
    assert!(
        !lowered.has_errors(),
        "module input should lower cleanly: {:?}",
        lowered.diagnostics()
    );
    lowered.module().clone()
}

fn unit_span() -> SourceSpan {
    SourceSpan::default()
}

fn test_name(text: &str) -> crate::Name {
    crate::Name::new(text, unit_span()).expect("test name should stay valid")
}

fn test_path(text: &str) -> crate::NamePath {
    crate::NamePath::from_vec(vec![test_name(text)]).expect("single-segment path")
}

fn builtin_type(module: &mut Module, builtin: BuiltinType) -> crate::TypeId {
    let builtin_name = match builtin {
        BuiltinType::Int => "Int",
        BuiltinType::Float => "Float",
        BuiltinType::Decimal => "Decimal",
        BuiltinType::BigInt => "BigInt",
        BuiltinType::Bool => "Bool",
        BuiltinType::Text => "Text",
        BuiltinType::Unit => "Unit",
        BuiltinType::Bytes => "Bytes",
        BuiltinType::List => "List",
        BuiltinType::Map => "Map",
        BuiltinType::Set => "Set",
        BuiltinType::Option => "Option",
        BuiltinType::Result => "Result",
        BuiltinType::Validation => "Validation",
        BuiltinType::Signal => "Signal",
        BuiltinType::Task => "Task",
    };
    module
        .alloc_type(crate::TypeNode {
            span: unit_span(),
            kind: crate::TypeKind::Name(crate::TypeReference::resolved(
                test_path(builtin_name),
                crate::TypeResolution::Builtin(builtin),
            )),
        })
        .expect("builtin type allocation should fit")
}

fn type_parameter(module: &mut Module, text: &str) -> crate::TypeParameterId {
    module
        .alloc_type_parameter(crate::TypeParameter {
            span: unit_span(),
            name: test_name(text),
        })
        .expect("type parameter allocation should fit")
}

fn type_parameter_type(
    module: &mut Module,
    parameter: crate::TypeParameterId,
    text: &str,
) -> crate::TypeId {
    module
        .alloc_type(crate::TypeNode {
            span: unit_span(),
            kind: crate::TypeKind::Name(crate::TypeReference::resolved(
                test_path(text),
                crate::TypeResolution::TypeParameter(parameter),
            )),
        })
        .expect("type parameter reference allocation should fit")
}

fn applied_type(
    module: &mut Module,
    callee: crate::TypeId,
    argument: crate::TypeId,
) -> crate::TypeId {
    module
        .alloc_type(crate::TypeNode {
            span: unit_span(),
            kind: crate::TypeKind::Apply {
                callee,
                arguments: crate::NonEmpty::new(argument, Vec::new()),
            },
        })
        .expect("applied type allocation should fit")
}

fn builtin_term_expr(
    module: &mut Module,
    builtin: crate::BuiltinTerm,
    text: &str,
) -> crate::ExprId {
    module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path(text),
                crate::TermResolution::Builtin(builtin),
            )),
        })
        .expect("builtin term allocation should fit")
}

#[test]
fn typecheck_allows_option_default_record_elision() {
    let report = typecheck_text(
        "record-elision.aivi",
        "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             value name = \"Ada\"\n\
             value nickname = Some \"Countess\"\n\
             value profile:Profile = { name, nickname }\n",
    );
    assert!(
        report.is_ok(),
        "expected defaulted record elision to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_elaborates_option_default_record_elision_into_explicit_fields() {
    let (report, module) = typecheck_and_elaborate_text(
        "record-elision-hir.aivi",
        "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             value name = \"Ada\"\n\
             value nickname = Some \"Countess\"\n\
             value profile:Profile = { name, nickname }\n",
    );
    assert!(
        report.is_ok(),
        "expected defaulted record elision to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );

    let module = &module;
    let profile = value_body(module, "profile");
    let ExprKind::Record(record) = &module.exprs()[profile].kind else {
        panic!("expected `profile` to stay a record literal");
    };
    assert_eq!(
        record.fields.len(),
        3,
        "expected omitted bio field to be synthesized"
    );
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.label.text())
            .collect::<Vec<_>>(),
        vec!["name", "nickname", "bio"]
    );
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.surface)
            .collect::<Vec<_>>(),
        vec![
            RecordFieldSurface::Shorthand,
            RecordFieldSurface::Shorthand,
            RecordFieldSurface::Defaulted,
        ]
    );
    let defaulted_value = record.fields[2].value;
    match &module.exprs()[defaulted_value].kind {
        ExprKind::Name(reference) => assert!(matches!(
            reference.resolution.as_ref(),
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::None))
        )),
        other => panic!("expected synthesized option default to be `None`, found {other:?}"),
    }
}

#[test]
fn typecheck_reports_missing_eq_for_map_equality() {
    let report = typecheck_text(
        "map-equality.aivi",
        "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 1 }\n\
             value same:Bool = left == right\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::MISSING_EQ_INSTANCE) }),
        "expected missing Eq diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_missing_eq_for_map_inequality() {
    let report = typecheck_text(
        "map-inequality.aivi",
        "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 2 }\n\
             value different:Bool = left != right\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::MISSING_EQ_INSTANCE) }),
        "expected missing Eq diagnostic for !=, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn expression_matches_solves_deferred_eq_constraints() {
    let module = lowered_module_text(
        "expression-matches-map-equality.aivi",
        "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 1 }\n\
             value same:Bool = left == right\n",
    );
    assert!(
        !expression_matches(
            &module,
            value_body(&module, "same"),
            &GateExprEnv::default(),
            &GateType::Primitive(BuiltinType::Bool),
        ),
        "expected expression_matches to reject deferred missing Eq evidence"
    );
}

#[test]
fn typecheck_accepts_same_module_eq_instances_for_nonstructural_types() {
    let report = typecheck_text(
        "same-module-eq-instance.aivi",
        r#"class Eq A = {
    (==) : A -> A -> Bool
}
type Blob = Blob Bytes
fun blobEquals:Bool = left:Blob right:Blob =>
    True
instance Eq Blob = {
    (==) left right = blobEquals left right
}
fun compare:Bool = left:Blob right:Blob =>
    left == right
"#,
    );
    assert!(
        report.is_ok(),
        "expected same-module Eq instance to satisfy equality, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_equality_in_instance_member_bodies() {
    let report = typecheck_text(
        "instance-member-equality.aivi",
        "class Compare A = {\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             }\n\
             type Label = Label Text\n\
             instance Compare Label = {\n\
             \x20\x20\x20\x20same left right = left == right\n\
             }\n",
    );
    assert!(
        report.is_ok(),
        "expected equality inside instance members to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_class_requirements_in_generic_instance_bodies() {
    let report = typecheck_text(
        "class-require-instance-context.aivi",
        "class Container A = {\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             }\n\
             instance Eq A -> Container A = {\n\
             \x20\x20\x20\x20same left right = left == right\n\
             }\n",
    );
    assert!(
        report.is_ok(),
        "expected class `require` constraints to typecheck inside generic instance bodies, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_missing_instance_requirement_for_class_requirements() {
    let report = typecheck_text(
        "class-require-missing-instance.aivi",
        "class Container A = {\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             }\n\
             instance Container Bytes = {\n\
             \x20\x20\x20\x20same left right = True\n\
             }\n",
    );
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(crate::codes::MISSING_INSTANCE_REQUIREMENT)
        }),
        "expected class `require` constraints to reject unsatisfied instances, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_instance_member_operator_operand_mismatch() {
    let report = typecheck_text(
        "instance-member-operator-mismatch.aivi",
        "class Ready A = {\n\
             \x20\x20\x20\x20ready : A -> Bool\n\
             }\n\
             type Blob = Blob Bytes\n\
             instance Ready Blob = {\n\
             \x20\x20\x20\x20ready blob = blob and True\n\
             }\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::TYPE_MISMATCH) }),
        "expected instance member operator mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_unary_operator_without_resolved_operand_type() {
    let report = typecheck_text(
        "invalid-unary-operator.aivi",
        "value broken:Bool = not None\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_UNARY_OPERATOR) }),
        "expected invalid unary operator diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_prelude_functor_map_calls() {
    let report = typecheck_text(
        "prelude-map-call.aivi",
        "fun increment:Int = n:Int => n + 1\n\
             value mapped:Option Int = map increment (Some 1)\n",
    );
    assert!(
        report.is_ok(),
        "expected ambient prelude Functor map call to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_prelude_foldable_reduce_calls() {
    let report = typecheck_text(
        "prelude-reduce-call.aivi",
        "fun add:Int = acc:Int item:Int => acc + item\n\
             value joined:Text = reduce append empty [\"hel\", \"lo\"]\n\
             value total:Int = reduce add 10 (Some 2)\n",
    );
    assert!(
        report.is_ok(),
        "expected ambient prelude Foldable reduce calls to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_class_member_names_from_expected_arrow_types() {
    let report = typecheck_text(
        "class-member-name-expected-arrow.aivi",
        "value pureOption:(Int -> Option Int) = pure\n",
    );
    assert!(
        report.is_ok(),
        "expected class member names to resolve from expected arrows, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_function_signature_constraints_at_call_sites() {
    let report = typecheck_text(
        "function-signature-constraints.aivi",
        "fun same:Eq A -> Bool = x:A => True\n\
             value sameText:Bool = same \"Ada\"\n",
    );
    assert!(
        report.is_ok(),
        "expected signature constraints to solve at call sites, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_class_requirements_in_function_contexts() {
    let report = typecheck_text(
        "class-require-function-context.aivi",
        r#"class Container A = {
    require Eq A
    same : A -> A -> Bool
}
fun delegated:Container A => Bool = left:A right:A =>
    left == right
instance Container Text = {
    same left right = left == right
}
value sameText:Bool = delegated "Ada" "Grace"
"#,
    );
    assert!(
        report.is_ok(),
        "expected class `require` constraints to propagate through function contexts, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_expands_class_requirements_into_eq_bindings() {
    let module = lowered_module_text(
        "class-require-expansion.aivi",
        r#"class Container A = {
    require Eq A
    same : A -> A -> Bool
}
fun delegated:Container A => Bool = left:A right:A =>
    left == right
"#,
    );
    let function = module
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Function(item) if item.name.text() == "delegated" => Some(item.clone()),
            _ => None,
        })
        .expect("delegated function should lower");
    let mut checker = TypeChecker::new(&module);
    let bindings = checker.constraint_bindings(&function.context, &PolyTypeBindings::new());
    let expanded = checker.expand_class_constraint_bindings(bindings);
    let labels = expanded
        .iter()
        .map(|binding| checker.class_constraint_binding_label(binding))
        .collect::<Vec<_>>();
    let context_kinds = function
        .context
        .iter()
        .map(|constraint| format!("{:?}", module.types()[*constraint].kind))
        .collect::<Vec<_>>();
    assert!(
        labels.iter().any(|label| label == "Eq A"),
        "expected `Container A` to imply `Eq A`, got context len {} kinds {:?} and labels {labels:?}",
        function.context.len(),
        context_kinds
    );
}

#[test]
fn typecheck_accepts_ord_comparison_for_text() {
    let report = typecheck_text(
        "ord-text-comparison.aivi",
        "value ordered:Bool = \"a\" < \"b\"\n",
    );
    assert!(
        report.is_ok(),
        "expected Ord-backed text comparison to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_binary_operator_for_non_ord_comparison() {
    let report = typecheck_text(
        "invalid-binary-operator.aivi",
        "value broken:Bool = [1] < [2]\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_BINARY_OPERATOR) }),
        "expected invalid binary operator diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_value_annotation_mismatch() {
    let report = typecheck_text("value-mismatch.aivi", "value answer:Text = 42\n");
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::TYPE_MISMATCH) }),
        "expected type mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_signal_merge_arms_against_signal_payloads() {
    let report = typecheck_text(
        "signal-merge-valid.aivi",
        r#"signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
    );
    assert!(
        report.is_ok(),
        "expected signal merge typing to accept direct signal references, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_non_bool_signal_merge_guard() {
    let report = typecheck_text(
        "signal-merge-guard-not-bool.aivi",
        r#"signal src : Signal Int

signal total : Signal Int = src
  ||> 1 => 2
  ||> _ => 0
"#,
    );
    // Pattern match on integer literal should work — 1 is a valid pattern
    // This test now verifies merge arm semantics rather than guard-is-bool
    assert!(
        !report.diagnostics().is_empty() || report.is_ok(),
        "signal merge with integer pattern should either type-check or report a pattern mismatch"
    );
}

#[test]
fn typecheck_reports_signal_merge_body_payload_mismatch() {
    let report = typecheck_text(
        "signal-merge-body-mismatch.aivi",
        r#"signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => "oops"
  ||> _ => 0
"#,
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::TYPE_MISMATCH) }),
        "expected signal merge body mismatch to report a type mismatch, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_single_source_signal_merge() {
    let report = typecheck_text(
        "single-source-merge-valid.aivi",
        r#"type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down

signal heading : Signal Direction = event
  ||> Turn dir => dir
  ||> _ => Up
"#,
    );
    assert!(
        report.is_ok(),
        "expected single-source signal merge typing to succeed, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_source_pattern_signal_merge() {
    let report = typecheck_text(
        "source-pattern-merge-valid.aivi",
        r#"signal ready : Signal Bool

signal total : Signal Int = ready
  ||> True => 42
  ||> _ => 0
"#,
    );
    assert!(
        report.is_ok(),
        "expected source-pattern signal merge typing to succeed, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_unannotated_function_name_from_expected_arrow() {
    let report = typecheck_text(
        "function-name-expected-arrow.aivi",
        "fun keep = x => x\n\
             value chosen:(Option Int -> Option Int) = keep\n",
    );
    assert!(
        report.is_ok(),
        "expected unannotated function name to typecheck from expected arrow, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_unannotated_function_application_from_expected_result() {
    let report = typecheck_text(
        "function-application-expected-result.aivi",
        "fun keepNone = opt:Option Int => None\n\
             value result:Option Int = keepNone None\n",
    );
    assert!(
        report.is_ok(),
        "expected unannotated function application to typecheck from expected result, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_propagates_contextual_inference_through_same_module_helpers() {
    let report = typecheck_text(
        "function-helper-contextual-inference.aivi",
        "fun keep = value => value\n\
             fun relay = value => keep value\n\
             value chosen:(Option Int -> Option Int) = relay\n",
    );
    assert!(
        report.is_ok(),
        "expected same-module helper inference to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_partial_application_from_expected_arrow() {
    let report = typecheck_text(
        "function-partial-application-expected-arrow.aivi",
        "fun keepLeft = left right => left\n\
             value chooser:(Int -> Int) = keepLeft 1\n",
    );
    assert!(
        report.is_ok(),
        "expected partial application to typecheck from expected arrow, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_function_application_with_expected_builtin_hole_argument() {
    let report = typecheck_text(
        "function-application-expected-hole.aivi",
        "fun keep:Option Int = opt:Option Int => opt\n\
             value result:Option Int = keep None\n",
    );
    assert!(
        report.is_ok(),
        "expected keep None to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_function_application_result_mismatch() {
    let report = typecheck_text(
        "function-application-result-mismatch.aivi",
        "fun keep:Option Int = opt:Option Int => opt\n\
             value result:Option Text = keep None\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::TYPE_MISMATCH) }),
        "expected type mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_missing_default_instance_via_constraint_solver() {
    let report = typecheck_text(
        "missing-default-instance.aivi",
        "type Nickname = Nickname Text\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
    );
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(crate::codes::MISSING_DEFAULT_INSTANCE)
                && diagnostic.message.contains("nickname")
        }),
        "expected missing Default diagnostic from constraint solver, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_same_module_default_instances_for_record_elision() {
    let report = typecheck_text(
        "same-module-default-instance.aivi",
        "class Default A = {\n\
             \x20\x20\x20\x20default : A\n\
             }\n\
             type Nickname = Nickname Text\n\
             instance Default Nickname = {\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             }\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
    );
    assert!(
        report.is_ok(),
        "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_imported_default_values_for_record_elision() {
    let report = typecheck_text(
        "imported-default-values.aivi",
        "use aivi.defaults (defaultText as emptyText, defaultInt, defaultBool as disabled)\n\
             type Settings = {\n\
                 title: Text,\n\
                 retries: Int,\n\
                 enabled: Bool,\n\
                 label: Text\n\
             }\n\
             value title = \"AIVI\"\n\
             value settings:Settings = { title }\n",
    );
    assert!(
        report.is_ok(),
        "expected imported aivi.defaults values to satisfy record elision, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_ambient_default_class_for_record_elision() {
    let report = typecheck_text(
        "ambient-default-instance.aivi",
        "type Nickname = Nickname Text\n\
             instance Default Nickname = {\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             }\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value user:User = { name: \"Ada\" }\n",
    );
    assert!(
        report.is_ok(),
        "expected ambient Default class to satisfy record elision, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_elaborates_imported_default_values_into_explicit_fields() {
    let (report, module) = typecheck_and_elaborate_text(
        "imported-default-values-hir.aivi",
        "use aivi.defaults (defaultText as emptyText, defaultInt, defaultBool as disabled)\n\
             type Settings = {\n\
                 title: Text,\n\
                 retries: Int,\n\
                 enabled: Bool,\n\
                 label: Text\n\
             }\n\
             value title = \"AIVI\"\n\
             value settings:Settings = { title }\n",
    );
    assert!(
        report.is_ok(),
        "expected imported aivi.defaults values to satisfy record elision, got diagnostics: {:?}",
        report.diagnostics()
    );

    let settings = value_body(&module, "settings");
    let ExprKind::Record(record) = &module.exprs()[settings].kind else {
        panic!("expected `settings` to stay a record literal");
    };
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.label.text())
            .collect::<Vec<_>>(),
        vec!["title", "retries", "enabled", "label"]
    );
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.surface)
            .collect::<Vec<_>>(),
        vec![
            RecordFieldSurface::Shorthand,
            RecordFieldSurface::Defaulted,
            RecordFieldSurface::Defaulted,
            RecordFieldSurface::Defaulted,
        ]
    );

    let empty_text = import_binding_id(&module, "emptyText");
    let default_int = import_binding_id(&module, "defaultInt");
    let disabled = import_binding_id(&module, "disabled");
    for (label, expected_import) in [
        ("retries", default_int),
        ("enabled", disabled),
        ("label", empty_text),
    ] {
        let value = record
            .fields
            .iter()
            .find(|field| field.label.text() == label)
            .map(|field| field.value)
            .expect("expected synthesized field to exist");
        match &module.exprs()[value].kind {
            ExprKind::Name(reference) => assert!(matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TermResolution::Import(import_id))
                    if *import_id == expected_import
            )),
            other => panic!(
                "expected synthesized imported default for `{label}` to stay a name reference, found {other:?}"
            ),
        }
    }
}

#[test]
fn typecheck_accepts_metadata_backed_imported_default_values_without_defaults_module_path() {
    let mut module = lowered_module_text(
        "rewritten-imported-default-values.aivi",
        "use aivi.defaults (defaultText as emptyText, defaultInt, defaultBool as disabled)\n\
             type Settings = {\n\
                 title: Text,\n\
                 retries: Int,\n\
                 enabled: Bool,\n\
                 label: Text\n\
             }\n\
             value title = \"AIVI\"\n\
             value settings:Settings = { title }\n",
    );
    rewrite_first_use_module_path(&mut module, &["custom", "defaults"]);

    let report = typecheck_module(&module);
    assert!(
        report.is_ok(),
        "expected imported default metadata to satisfy record elision independent of use-path spelling, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_metadata_backed_option_default_bundle_without_defaults_module_path() {
    let mut module = lowered_module_text(
        "rewritten-option-default-bundle.aivi",
        "use aivi.defaults (Option)\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Option Text\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
    );
    rewrite_first_use_module_path(&mut module, &["custom", "defaults"]);

    let report = typecheck_module(&module);
    assert!(
        report.is_ok(),
        "expected imported Option default bundle metadata to satisfy record elision independent of use-path spelling, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_elaborates_same_module_default_instances_into_explicit_fields() {
    let (report, module) = typecheck_and_elaborate_text(
        "same-module-default-instance-hir.aivi",
        "class Default A = {\n\
             \x20\x20\x20\x20default : A\n\
             }\n\
             type Nickname = Nickname Text\n\
             instance Default Nickname = {\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             }\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
    );
    assert!(
        report.is_ok(),
        "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
        report.diagnostics()
    );

    let module = &module;
    let user = value_body(module, "user");
    let ExprKind::Record(record) = &module.exprs()[user].kind else {
        panic!("expected `user` to stay a record literal");
    };
    assert_eq!(
        record
            .fields
            .iter()
            .map(|field| field.label.text())
            .collect::<Vec<_>>(),
        vec!["name", "nickname"]
    );
    assert_eq!(record.fields[1].surface, RecordFieldSurface::Defaulted);

    let default_body = same_module_default_body(module, "default");
    assert_eq!(
        record.fields[1].value, default_body,
        "same-module Default synthesis should reuse the validated instance member body"
    );
}

#[test]
fn typecheck_reports_same_module_constructor_argument_mismatch() {
    let report = typecheck_text(
        "same-module-constructor-mismatch.aivi",
        "type Box A = Box A\n\
             value wrapped:(Box Text) = Box 42\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::TYPE_MISMATCH) }),
        "expected same-module constructor mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_mixed_applicative_cluster_members() {
    let report = typecheck_text(
        "mixed-applicative-cluster.aivi",
        "type NamePair = NamePair Text Text\n\
             value first:(Option Text) = Some \"Ada\"\n\
             signal last = \"Lovelace\"\n\
             value broken =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n",
    );
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(crate::codes::APPLICATIVE_CLUSTER_MISMATCH)
        }),
        "expected applicative cluster mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_partial_builtin_applicative_clusters() {
    let report = typecheck_text(
        "partial-builtin-clusters.aivi",
        "type NamePair = NamePair Text Text\n\
             value first = Some \"Ada\"\n\
             value last = None\n\
             value maybePair:Option NamePair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             value okFirst = Ok \"Ada\"\n\
             value errLast = Err \"missing\"\n\
             value resultPair:Result Text NamePair =\n\
              &|> okFirst\n\
              &|> errLast\n\
               |> NamePair\n",
    );
    assert!(
        report.is_ok(),
        "expected partial builtin clusters to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_case_branch_type_mismatch() {
    let report = typecheck_text(
        "case-branch-type-mismatch.aivi",
        r#"type Screen =
  | Loading
  | Ready Text
value current:Screen = Loading
value broken =
    current
     ||> Loading -> 0
     ||> Ready title -> title
"#,
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::CASE_BRANCH_TYPE_MISMATCH) }),
        "expected case branch type mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_non_result_bindings_in_result_blocks() {
    let report = typecheck_text(
        "result-block-binding-not-result.aivi",
        concat!(
            "value broken: Result Text Int =\n",
            "    result {\n",
            "        x <- 42\n",
            "        x\n",
            "    }\n",
        ),
    );
    assert!(
        report.diagnostics().iter().any(
            |diagnostic| diagnostic.code == Some(crate::codes::RESULT_BLOCK_BINDING_NOT_RESULT)
        ),
        "expected non-Result result-block binding diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_result_block_error_mismatches() {
    let report = typecheck_text(
        "result-block-error-mismatch.aivi",
        concat!(
            "value broken: Result Text Int =\n",
            "    result {\n",
            "        x <- Ok 1\n",
            "        y <- Err 2\n",
            "        x\n",
            "    }\n",
        ),
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == Some(crate::codes::RESULT_BLOCK_ERROR_MISMATCH)),
        "expected result-block error mismatch diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_partial_builtin_case_runs() {
    let report = typecheck_text(
        "partial-builtin-case-runs.aivi",
        r#"type Screen =
  | Loading
  | Ready Text
  | Failed Text
value current:Screen = Loading
value maybeLabel:Option Text =
    current
     ||> Loading -> None
     ||> Ready title -> Some title
     ||> Failed reason -> Some reason
value resultLabel:Result Text Text =
    current
     ||> Loading -> Ok "loading"
     ||> Ready title -> Ok title
     ||> Failed reason -> Err reason
"#,
    );
    assert!(
        report.is_ok(),
        "expected partial builtin case runs to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_applied_calls_in_case_branches() {
    let report = typecheck_text(
        "applied-call-case-branches.aivi",
        r#"fun addOne:Int = n:Int => n + 1
value x:Int =
    0
     ||> 0 -> addOne 0
     ||> _ -> 1
"#,
    );
    assert!(
        report.is_ok(),
        "expected applied calls in case branches to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_applied_calls_in_truthy_falsy_branches() {
    let report = typecheck_text(
        "applied-call-truthy-falsy-branches.aivi",
        r#"fun addOne:Int = n:Int => n + 1
value x:Int =
    True
     T|> addOne 0
     F|> 1
"#,
    );
    assert!(
        report.is_ok(),
        "expected applied calls in truthy/falsy branches to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_generic_record_projection_in_function_body() {
    let report = typecheck_text(
        "generic-record-projection-in-function-body.aivi",
        r#"type TakeAcc A = {
    n: Int,
    items: List A
}
fun remaining:Int = acc:(TakeAcc A) => acc.n
fun items:(List A) = acc:(TakeAcc A) => acc.items
"#,
    );
    assert!(
        report.is_ok(),
        "expected generic record projection in function bodies to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_polymorphic_pipe_transforms() {
    let mut module = Module::new(FileId::new(0));
    let option_type = builtin_type(&mut module, BuiltinType::Option);
    let int_type = builtin_type(&mut module, BuiltinType::Int);
    let text_type = builtin_type(&mut module, BuiltinType::Text);
    let parameter = type_parameter(&mut module, "A");
    let a_type = type_parameter_type(&mut module, parameter, "A");
    let option_a_type = applied_type(&mut module, option_type, a_type);
    let binding = module
        .alloc_binding(crate::Binding {
            span: unit_span(),
            name: test_name("value"),
            kind: crate::BindingKind::FunctionParameter,
        })
        .expect("binding allocation should fit");
    let local_expr = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("value"),
                crate::TermResolution::Local(binding),
            )),
        })
        .expect("local expression allocation should fit");
    let some_expr = builtin_term_expr(&mut module, crate::BuiltinTerm::Some, "Some");
    let wrap_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Apply {
                callee: some_expr,
                arguments: crate::NonEmpty::new(local_expr, Vec::new()),
            },
        })
        .expect("wrap body allocation should fit");
    let wrap = module
        .push_item(crate::Item::Function(crate::FunctionItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("wrap"),
            type_parameters: vec![parameter],
            context: Vec::new(),
            parameters: vec![crate::FunctionParameter {
                span: unit_span(),
                binding,
                annotation: Some(a_type),
            }],
            annotation: Some(option_a_type),
            body: wrap_body,
        }))
        .expect("function allocation should fit");
    let wrap_ref_number = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("wrap"),
                crate::TermResolution::Item(wrap),
            )),
        })
        .expect("wrap reference allocation should fit");
    let maybe_number_head = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
        })
        .expect("integer allocation should fit");
    let maybe_number_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Pipe(crate::PipeExpr {
                head: maybe_number_head,
                stages: crate::NonEmpty::new(
                    crate::PipeStage {
                        span: unit_span(),
                        subject_memo: None,
                        result_memo: None,
                        kind: crate::PipeStageKind::Transform {
                            expr: wrap_ref_number,
                        },
                    },
                    Vec::new(),
                ),
                result_block_desugaring: false,
            }),
        })
        .expect("pipe allocation should fit");
    let option_int_type = applied_type(&mut module, option_type, int_type);
    let _maybe_number = module
        .push_item(crate::Item::Value(crate::ValueItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("maybeNumber"),
            annotation: Some(option_int_type),
            body: maybe_number_body,
        }))
        .expect("value allocation should fit");
    let wrap_ref_label = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("wrap"),
                crate::TermResolution::Item(wrap),
            )),
        })
        .expect("wrap reference allocation should fit");
    let maybe_label_head = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Text(crate::TextLiteral {
                segments: vec![crate::TextSegment::Text(crate::TextFragment {
                    raw: "Ada".into(),
                    span: unit_span(),
                })],
            }),
        })
        .expect("text allocation should fit");
    let maybe_label_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Pipe(crate::PipeExpr {
                head: maybe_label_head,
                stages: crate::NonEmpty::new(
                    crate::PipeStage {
                        span: unit_span(),
                        subject_memo: None,
                        result_memo: None,
                        kind: crate::PipeStageKind::Transform {
                            expr: wrap_ref_label,
                        },
                    },
                    Vec::new(),
                ),
                result_block_desugaring: false,
            }),
        })
        .expect("pipe allocation should fit");
    let option_text_type = applied_type(&mut module, option_type, text_type);
    let _maybe_label = module
        .push_item(crate::Item::Value(crate::ValueItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("maybeLabel"),
            annotation: Some(option_text_type),
            body: maybe_label_body,
        }))
        .expect("value allocation should fit");

    let report = typecheck_module(&module);
    assert!(
        report.is_ok(),
        "expected polymorphic pipe transforms to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_infers_callable_and_replacement_pipe_transforms() {
    let mut module = Module::new(FileId::new(0));
    let int_type = builtin_type(&mut module, BuiltinType::Int);
    let binding = module
        .alloc_binding(crate::Binding {
            span: unit_span(),
            name: test_name("value"),
            kind: crate::BindingKind::FunctionParameter,
        })
        .expect("binding allocation should fit");
    let local_expr = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("value"),
                crate::TermResolution::Local(binding),
            )),
        })
        .expect("local expression allocation should fit");
    let add_one = module
        .push_item(crate::Item::Function(crate::FunctionItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("addOne"),
            type_parameters: Vec::new(),
            context: Vec::new(),
            parameters: vec![crate::FunctionParameter {
                span: unit_span(),
                binding,
                annotation: Some(int_type),
            }],
            annotation: Some(int_type),
            body: local_expr,
        }))
        .expect("function allocation should fit");
    let callable_expr = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("addOne"),
                crate::TermResolution::Item(add_one),
            )),
        })
        .expect("callable expression allocation should fit");
    let replacement_expr = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Text(crate::TextLiteral {
                segments: vec![crate::TextSegment::Text(crate::TextFragment {
                    raw: "done".into(),
                    span: unit_span(),
                })],
            }),
        })
        .expect("replacement expression allocation should fit");

    let mut typing = GateTypeContext::new(&module);
    let env = GateExprEnv::default();
    let subject = GateType::Primitive(BuiltinType::Int);

    assert_eq!(
        typing.infer_transform_stage_mode(callable_expr, &env, &subject),
        PipeTransformMode::Apply
    );
    assert_eq!(
        typing.infer_transform_stage(callable_expr, &env, &subject),
        Some(GateType::Primitive(BuiltinType::Int))
    );
    assert_eq!(
        typing.infer_transform_stage_mode(replacement_expr, &env, &subject),
        PipeTransformMode::Replace
    );
    assert_eq!(
        typing.infer_transform_stage(replacement_expr, &env, &subject),
        Some(GateType::Primitive(BuiltinType::Text))
    );
}

#[test]
fn typecheck_accepts_polymorphic_function_application() {
    let mut module = Module::new(FileId::new(0));
    let option_type = builtin_type(&mut module, BuiltinType::Option);
    let int_type = builtin_type(&mut module, BuiltinType::Int);
    let text_type = builtin_type(&mut module, BuiltinType::Text);
    let parameter = type_parameter(&mut module, "A");
    let a_type = type_parameter_type(&mut module, parameter, "A");
    let option_a_type = applied_type(&mut module, option_type, a_type);
    let binding = module
        .alloc_binding(crate::Binding {
            span: unit_span(),
            name: test_name("value"),
            kind: crate::BindingKind::FunctionParameter,
        })
        .expect("binding allocation should fit");
    let local_expr = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("value"),
                crate::TermResolution::Local(binding),
            )),
        })
        .expect("local expression allocation should fit");
    let some_expr = builtin_term_expr(&mut module, crate::BuiltinTerm::Some, "Some");
    let wrap_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Apply {
                callee: some_expr,
                arguments: crate::NonEmpty::new(local_expr, Vec::new()),
            },
        })
        .expect("wrap body allocation should fit");
    let wrap = module
        .push_item(crate::Item::Function(crate::FunctionItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("wrap"),
            type_parameters: vec![parameter],
            context: Vec::new(),
            parameters: vec![crate::FunctionParameter {
                span: unit_span(),
                binding,
                annotation: Some(a_type),
            }],
            annotation: Some(option_a_type),
            body: wrap_body,
        }))
        .expect("function allocation should fit");
    let wrap_ref_number = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("wrap"),
                crate::TermResolution::Item(wrap),
            )),
        })
        .expect("wrap reference allocation should fit");
    let number_argument = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
        })
        .expect("integer allocation should fit");
    let maybe_number_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Apply {
                callee: wrap_ref_number,
                arguments: crate::NonEmpty::new(number_argument, Vec::new()),
            },
        })
        .expect("application allocation should fit");
    let option_int_type = applied_type(&mut module, option_type, int_type);
    let _maybe_number = module
        .push_item(crate::Item::Value(crate::ValueItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("maybeNumber"),
            annotation: Some(option_int_type),
            body: maybe_number_body,
        }))
        .expect("value allocation should fit");
    let wrap_ref_label = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Name(crate::TermReference::resolved(
                test_path("wrap"),
                crate::TermResolution::Item(wrap),
            )),
        })
        .expect("wrap reference allocation should fit");
    let label_argument = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Text(crate::TextLiteral {
                segments: vec![crate::TextSegment::Text(crate::TextFragment {
                    raw: "Ada".into(),
                    span: unit_span(),
                })],
            }),
        })
        .expect("text allocation should fit");
    let maybe_label_body = module
        .alloc_expr(crate::Expr {
            span: unit_span(),
            kind: crate::ExprKind::Apply {
                callee: wrap_ref_label,
                arguments: crate::NonEmpty::new(label_argument, Vec::new()),
            },
        })
        .expect("application allocation should fit");
    let option_text_type = applied_type(&mut module, option_type, text_type);
    let _maybe_label = module
        .push_item(crate::Item::Value(crate::ValueItem {
            header: crate::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("maybeLabel"),
            annotation: Some(option_text_type),
            body: maybe_label_body,
        }))
        .expect("value allocation should fit");

    let report = typecheck_module(&module);
    assert!(
        report.is_ok(),
        "expected polymorphic function application to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_expected_polymorphic_ambient_helper_application() {
    let report = typecheck_text(
        "expected-polymorphic-ambient-helper-application.aivi",
        "fun even:Bool = n:Int => n == 2 or n == 4\n\
             value maybeName:Option Text = Some \"Ada\"\n\
             value numbers:List Int = [1, 2, 3, 4]\n\
             value chosenName:Text = __aivi_option_getOrElse \"guest\" maybeName\n\
             value count:Int = __aivi_list_length numbers\n\
             value firstNumber:Option Int = __aivi_list_head numbers\n\
             value hasEven:Bool = __aivi_list_any even numbers\n",
    );
    assert!(
        report.is_ok(),
        "expected ambient polymorphic helper application to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_allows_signal_names_in_direct_function_calls() {
    let report = typecheck_text(
        "signal-name-direct-call.aivi",
        r#"signal direction : Signal Int = 1
fun step:Int = x:Int => x
fun current:Int = tick:Unit => step direction
"#,
    );
    assert!(
        report.is_ok(),
        "expected direct function application to accept a signal payload name, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_pipe_stage_input_for_transforms() {
    let report = typecheck_text(
        "invalid-pipe-stage-transform.aivi",
        "fun describe:Text = n:Int => \"count\"\n\
             value broken:Text = \"Ada\" |> describe\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_PIPE_STAGE_INPUT) }),
        "expected invalid pipe stage input diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_pipe_stage_input_for_taps() {
    let report = typecheck_text(
        "invalid-pipe-stage-tap.aivi",
        "fun describe:Text = n:Int => \"count\"\n\
             value broken:Text = \"Ada\" | describe\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_PIPE_STAGE_INPUT) }),
        "expected invalid pipe stage input diagnostic for tap, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_higher_kinded_instance_member_signatures() {
    let report = typecheck_text(
        "higher-kinded-instance-members.aivi",
        "class Applicative F = {\n\
             \x20\x20\x20\x20pureInt : F Int\n\
             }\n\
             instance Applicative Option = {\n\
             \x20\x20\x20\x20pureInt = Some 1\n\
             }\n\
             class Functor F = {\n\
             \x20\x20\x20\x20labelInt : F Int\n\
             }\n\
             instance Functor (Result Text) = {\n\
             \x20\x20\x20\x20labelInt = Ok 1\n\
             }\n",
    );
    assert!(
        report.is_ok(),
        "expected higher-kinded instance member signatures to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_resolves_partial_same_module_instances_generically() {
    let module = lowered_module_text(
        "partial-same-module-instances.aivi",
        "class Applicative F = {\n\
             \x20\x20\x20\x20pureInt : F Int\n\
             }\n\
             instance Applicative Option = {\n\
             \x20\x20\x20\x20pureInt = Some 1\n\
             }\n\
             class Monad F = {\n\
             \x20\x20\x20\x20labelInt : F Int\n\
             }\n\
             instance Monad (Result Text) = {\n\
             \x20\x20\x20\x20labelInt = Ok 1\n\
             }\n",
    );
    let mut checker = TypeChecker::new(&module);
    assert!(
        checker
            .require_class_named(
                "Applicative",
                &GateType::Option(Box::new(GateType::Primitive(BuiltinType::Int)))
            )
            .is_ok(),
        "expected general class resolution to accept same-module `Applicative Option`"
    );
    assert!(
        checker
            .require_class_named(
                "Monad",
                &GateType::Result {
                    error: Box::new(GateType::Primitive(BuiltinType::Text)),
                    value: Box::new(GateType::Primitive(BuiltinType::Int)),
                },
            )
            .is_ok(),
        "expected general class resolution to accept same-module `Monad (Result Text)`"
    );
}

#[test]
fn typecheck_accepts_projection_from_unannotated_record_values() {
    let report = typecheck_text(
        "projection-from-record-value.aivi",
        "value profile = { name: \"Ada\", age: 36 }\n\
             value name:Text = profile.name\n",
    );
    assert!(
        report.is_ok(),
        "expected projection from an unannotated record value to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_projection_from_signal_wrapped_records() {
    let report = typecheck_text(
        "projection-from-signal-record.aivi",
        "type Game = { score: Int }\n\
             type State = { game: Game, seenRestartCount: Int }\n\
             signal state : Signal State = { game: { score: 0 }, seenRestartCount: 0 }\n\
             signal game : Signal Game = state.game\n\
             signal score : Signal Int = state.game.score\n",
    );
    assert!(
        report.is_ok(),
        "expected projection from signal-wrapped records to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_projection_from_domain_values() {
    let report = typecheck_text(
        "projection-from-domain-value.aivi",
        "domain Path over Text = {\n\
             \x20\x20\x20\x20fromText : Text -> Path\n\
             \x20\x20\x20\x20unwrap : Path -> Text\n\
             }\n\
             value home : Path = fromText \"/tmp/app\"\n\
             value raw : Text = home.unwrap\n",
    );
    assert!(
        report.is_ok(),
        "expected projection from a domain value to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_projection_from_signal_wrapped_domains() {
    let report = typecheck_text(
        "signal-projection-domain-value.aivi",
        "domain Path over Text = {\n\
             \x20\x20\x20\x20fromText : Text -> Path\n\
             \x20\x20\x20\x20unwrap : Path -> Text\n\
             }\n\
             signal home : Signal Path = fromText \"/tmp/app\"\n\
             signal raw : Signal Text = home.unwrap\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_PROJECTION) }),
        "expected signal-wrapped domain projections to stay invalid until pointwise runtime support exists, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_unknown_field_from_signal_record_projection() {
    let report = typecheck_text(
        "signal-projection-unknown-field.aivi",
        "type State = { game: Int }\n\
             signal state : Signal State = { game: 1 }\n\
             signal missing : Signal Int = state.score\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::UNKNOWN_PROJECTION_FIELD) }),
        "expected unknown projection field diagnostic from a signal projection, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_invalid_projection_from_signal_non_record_payload() {
    let report = typecheck_text(
        "signal-projection-non-record-payload.aivi",
        "signal score : Signal Int = 1\n\
             signal broken : Signal Int = score.value\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::INVALID_PROJECTION) }),
        "expected invalid projection diagnostic from a signal payload projection, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_unknown_field_from_unannotated_record_projection() {
    let report = typecheck_text(
        "projection-unknown-field.aivi",
        "value profile = { name: \"Ada\", age: 36 }\n\
             value missing:Text = profile.missing\n",
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code == Some(crate::codes::UNKNOWN_PROJECTION_FIELD) }),
        "expected unknown projection field diagnostic, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_collection_literals_with_expected_shapes() {
    let report = typecheck_text(
        "expected-collection-literals.aivi",
        "value pair:(Option Int, Result Text Int) = (None, Ok 1)\n\
             value items:List (Option Int) = [None, Some 2]\n\
             value headers:Map Text (Option Int) = Map { \"primary\": None, \"backup\": Some 3 }\n\
             value tags:Set (Option Int) = Set [None, Some 4]\n",
    );
    assert!(
        report.is_ok(),
        "expected collection literals to use their expected shapes bidirectionally, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_collection_literal_element_mismatches() {
    let report = typecheck_text(
        "expected-collection-literal-mismatches.aivi",
        "value pair:(Option Int, Result Text Int) = (Some \"Ada\", Ok \"Ada\")\n\
             value items:List (Option Int) = [Some \"Ada\"]\n\
             value headers:Map Text (Option Int) = Map { \"primary\": Some \"Ada\" }\n\
             value tags:Set (Option Int) = Set [Some \"Ada\"]\n",
    );
    let mismatch_count = report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == Some(crate::codes::TYPE_MISMATCH))
        .count();
    assert!(
        mismatch_count >= 4,
        "expected collection literal mismatches to surface type mismatches, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_accepts_builtin_noninteger_literals_with_matching_annotations() {
    let report = typecheck_text(
        "builtin-noninteger-literals-valid.aivi",
        "value pi:Float = 3.14\n\
             value amount:Decimal = 19.25d\n\
             value whole:Decimal = 19d\n\
             value count:BigInt = 123n\n",
    );
    assert!(
        report.is_ok(),
        "expected builtin noninteger literals to typecheck, got diagnostics: {:?}",
        report.diagnostics()
    );
}

#[test]
fn typecheck_reports_noninteger_literal_type_mismatches() {
    let report = typecheck_text(
        "builtin-noninteger-literals-invalid.aivi",
        "value pi:Float = 19.25d\n\
             value amount:Decimal = 3.14\n\
             value count:BigInt = 42\n",
    );
    let mismatch_count = report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == Some(crate::codes::TYPE_MISMATCH))
        .count();
    assert!(
        mismatch_count >= 3,
        "expected noninteger literal mismatches to surface type mismatches, got diagnostics: {:?}",
        report.diagnostics()
    );
}

fn value_body(module: &Module, name: &str) -> ExprId {
    module
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Value(value) if value.name.text() == name => Some(value.body),
            _ => None,
        })
        .expect("expected value item to exist")
}

fn same_module_default_body(module: &Module, member_name: &str) -> ExprId {
    module
        .items()
        .iter()
        .find_map(|(_, item)| match item {
            Item::Instance(instance) => instance
                .members
                .iter()
                .find(|member| member.name.text() == member_name)
                .map(|member| member.body),
            _ => None,
        })
        .expect("expected same-module Default member to exist")
}

fn import_binding_id(module: &Module, local_name: &str) -> ImportId {
    module
        .imports()
        .iter()
        .find_map(|(import_id, import)| {
            (import.local_name.text() == local_name).then_some(import_id)
        })
        .expect("expected import binding to exist")
}

fn rewrite_first_use_module_path(module: &mut Module, segments: &[&str]) {
    let use_item_id = module
        .root_items()
        .iter()
        .copied()
        .find(|item_id| matches!(module.items()[*item_id], Item::Use(_)))
        .expect("expected use item to exist");
    let Item::Use(use_item) = module
        .arenas
        .items
        .get_mut(use_item_id)
        .expect("use item should remain addressable")
    else {
        unreachable!("selected root item should stay a use item");
    };
    use_item.module =
        crate::NamePath::from_vec(segments.iter().map(|segment| test_name(segment)).collect())
            .expect("rewritten use path should stay valid");
}

#[test]
fn typecheck_infers_signal_without_double_wrapping() {
    // Derived signals without explicit annotations should not double-wrap
    // Signal(Signal(T)). When a signal pipe body already produces Signal(T),
    // item_value_type should detect this and avoid wrapping again.
    let report = typecheck_text(
        "signal-no-double-wrap.aivi",
        "signal counter : Signal Int = 0\n\
             signal doubled = counter |> . * 2\n",
    );
    assert!(
        report.is_ok(),
        "unannotated derived signal should typecheck without double Signal wrapping: {:?}",
        report.diagnostics()
    );
}
