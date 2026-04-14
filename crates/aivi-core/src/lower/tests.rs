use std::{collections::HashSet, fs, path::PathBuf};

use aivi_base::{FileId, SourceDatabase, SourceSpan};
use aivi_hir::{
    BuiltinType, ImportModuleResolution, ImportResolver, PipeTransformMode, exports,
    lower_module_with_resolver, resolver::RawHoistItem,
};
use aivi_syntax::parse_module;

use super::{
    LoweringError, RuntimeFragmentSpec, lower_module, lower_runtime_fragment,
    lower_runtime_module_with_workspace, validate_general_expr_report_completeness,
};
use crate::{
    BuiltinApplicativeCarrier, BuiltinApplyCarrier, BuiltinBifunctorCarrier,
    BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier, BuiltinFoldableCarrier,
    BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject, BuiltinTraversableCarrier,
    DecodeStep, GateStage, ItemKind, Reference, StageKind, Type,
    validate::{ValidationError, validate_module},
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    aivi_hir::lower_module(&parsed.module)
}

fn expect_executable_evidence_item(core: &crate::Module, expr_id: crate::ExprId) -> crate::ItemId {
    let crate::ExprKind::Reference(Reference::ExecutableEvidence(item)) =
        &core.exprs()[expr_id].kind
    else {
        panic!("expected executable evidence item reference");
    };
    *item
}

fn expect_builtin_evidence_item(
    core: &crate::Module,
    expr_id: crate::ExprId,
    expected: BuiltinClassMemberIntrinsic,
) -> crate::ItemId {
    let item = expect_executable_evidence_item(core, expr_id);
    let body = core.items()[item]
        .body
        .expect("builtin evidence item should carry a body");
    match &core.exprs()[body].kind {
        crate::ExprKind::Reference(Reference::BuiltinClassMember(intrinsic)) => {
            assert_eq!(*intrinsic, expected);
            assert!(
                core.items()[item].parameters.is_empty(),
                "zero-arity builtin evidence should lower as a value item"
            );
        }
        crate::ExprKind::Apply { callee, arguments } => {
            let crate::ExprKind::Reference(Reference::BuiltinClassMember(intrinsic)) =
                &core.exprs()[*callee].kind
            else {
                panic!("builtin evidence body should call a builtin class member");
            };
            assert_eq!(*intrinsic, expected);
            assert_eq!(
                arguments.len(),
                core.items()[item].parameters.len(),
                "builtin evidence wrapper should forward each synthetic parameter"
            );
        }
        other => {
            panic!("builtin evidence item should lower to a builtin wrapper body, found {other:?}")
        }
    }
    item
}

fn find_core_item(core: &crate::Module, name: &str) -> crate::ItemId {
    core.items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("expected core item `{name}`"))
}

fn expect_value_apply_callee(core: &crate::Module, name: &str) -> crate::ExprId {
    let owner = find_core_item(core, name);
    let body = core.items()[owner]
        .body
        .unwrap_or_else(|| panic!("core item `{name}` should carry a lowered body"));
    let crate::ExprKind::Apply { callee, .. } = &core.exprs()[body].kind else {
        panic!("core item `{name}` should lower to an apply expression");
    };
    *callee
}

struct SingleImportResolver {
    module_path: Vec<&'static str>,
    module_file: &'static str,
    module_text: &'static str,
}

impl ImportResolver for SingleImportResolver {
    fn resolve(&self, path: &[&str]) -> ImportModuleResolution {
        if path != self.module_path {
            return ImportModuleResolution::Missing;
        }
        let lowered = lower_text(self.module_file, self.module_text);
        if lowered.has_errors() {
            return ImportModuleResolution::Missing;
        }
        ImportModuleResolution::Resolved(exports(lowered.module()))
    }

    fn workspace_hoist_items(&self) -> Vec<RawHoistItem> {
        Vec::new()
    }
}

fn lower_text_with_single_import(
    path: &str,
    text: &str,
    import_path: Vec<&'static str>,
    import_file: &'static str,
    import_text: &'static str,
) -> aivi_hir::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let resolver = SingleImportResolver {
        module_path: import_path,
        module_file: import_file,
        module_text: import_text,
    };
    lower_module_with_resolver(&parsed.module, Some(&resolver))
}

fn lower_fixture(path: &str) -> aivi_hir::LoweringResult {
    let text = fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
    lower_text(path, &text)
}

fn unit_span() -> SourceSpan {
    SourceSpan::default()
}

fn test_name(text: &str) -> aivi_hir::Name {
    aivi_hir::Name::new(text, unit_span()).expect("test name should stay valid")
}

fn test_path(text: &str) -> aivi_hir::NamePath {
    aivi_hir::NamePath::from_vec(vec![test_name(text)]).expect("single-segment path")
}

fn builtin_type(module: &mut aivi_hir::Module, builtin: BuiltinType) -> aivi_hir::TypeId {
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
        .alloc_type(aivi_hir::TypeNode {
            span: unit_span(),
            kind: aivi_hir::TypeKind::Name(aivi_hir::TypeReference::resolved(
                test_path(builtin_name),
                aivi_hir::TypeResolution::Builtin(builtin),
            )),
        })
        .expect("builtin type allocation should fit")
}

#[test]
fn lowers_pipe_and_source_fixtures_into_core_ir() {
    let lowered = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
    assert!(
        !lowered.has_errors(),
        "gate fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let maybe_active = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "maybeActive")
        .map(|(id, _)| id)
        .expect("expected maybeActive item");
    let pipes = &core.items()[maybe_active].pipes;
    assert_eq!(pipes.len(), 1);
    let pipe = &core.pipes()[pipes[0]];
    let first_stage = &core.stages()[pipe.stages[0]];
    assert!(matches!(
        &first_stage.kind,
        StageKind::Gate(GateStage::Ordinary { .. })
    ));
    let pretty = core.pretty();
    assert!(
        pretty.contains("gate"),
        "pretty dump should mention gate stages: {pretty}"
    );
}

#[test]
fn lower_module_preserves_pipe_memos_in_core_pipe_exprs() {
    let lowered = lower_text(
        "pipe-memos.aivi",
        r#"
fun add1:Int = x:Int=>    x + 1

fun demo:Int = x:Int=> x  |> #before add1 #after
  |> before + after
"#,
    );
    assert!(
        !lowered.has_errors(),
        "pipe memo fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("typed-core module should validate");

    let demo = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "demo")
        .map(|(id, _)| id)
        .expect("expected demo item");
    let body = core.items()[demo]
        .body
        .expect("demo function should have a typed-core body");
    let crate::expr::ExprKind::Pipe(pipe) = &core.exprs()[body].kind else {
        panic!("expected demo body to lower into a pipe expression");
    };
    assert_eq!(pipe.stages.len(), 2);
    assert!(pipe.stages[0].supports_memos());
    assert!(pipe.stages[0].subject_memo.is_some());
    assert!(pipe.stages[0].result_memo.is_some());
    assert_eq!(pipe.stages[1].subject_memo, None);
    assert_eq!(pipe.stages[1].result_memo, None);
}

#[test]
fn lower_module_preserves_grouped_pipe_memos_in_core_pipe_exprs() {
    let lowered = lower_text(
        "grouped-pipe-memos.aivi",
        r#"
type StageChoice = Ready Int | Missing

fun caseDemo:Int = input:StageChoice=> input
 ||> Ready value -> value + 1 #resolved
 ||> Missing -> 0 #resolved
 |> resolved

fun truthyDemo:Int = input:Option Int=> input
 T|> . + 1 #branch
 F|> 0 #branch
 |> branch
"#,
    );
    assert!(
        !lowered.has_errors(),
        "grouped pipe memo fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("typed-core module should validate");

    let case_demo = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "caseDemo")
        .map(|(id, _)| id)
        .expect("expected caseDemo item");
    let case_body = core.items()[case_demo]
        .body
        .expect("caseDemo should have a typed-core body");
    let crate::expr::ExprKind::Pipe(case_pipe) = &core.exprs()[case_body].kind else {
        panic!("expected caseDemo to lower into a pipe expression");
    };
    assert!(matches!(
        case_pipe.stages[0].kind,
        crate::expr::PipeStageKind::Case { .. }
    ));
    assert!(case_pipe.stages[0].result_memo.is_some());

    let truthy_demo = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "truthyDemo")
        .map(|(id, _)| id)
        .expect("expected truthyDemo item");
    let truthy_body = core.items()[truthy_demo]
        .body
        .expect("truthyDemo should have a typed-core body");
    let crate::expr::ExprKind::Pipe(truthy_pipe) = &core.exprs()[truthy_body].kind else {
        panic!("expected truthyDemo to lower into a pipe expression");
    };
    assert!(matches!(
        truthy_pipe.stages[0].kind,
        crate::expr::PipeStageKind::TruthyFalsy(_)
    ));
    assert!(truthy_pipe.stages[0].result_memo.is_some());
}

#[test]
fn lowers_transform_stage_modes_into_core_pipe_nodes() {
    let mut module = aivi_hir::Module::new(FileId::new(0));
    let int_type = builtin_type(&mut module, BuiltinType::Int);
    let text_type = builtin_type(&mut module, BuiltinType::Text);
    let binding = module
        .alloc_binding(aivi_hir::Binding {
            span: unit_span(),
            name: test_name("value"),
            kind: aivi_hir::BindingKind::FunctionParameter,
        })
        .expect("binding allocation should fit");
    let local_expr = module
        .alloc_expr(aivi_hir::Expr {
            span: unit_span(),
            kind: aivi_hir::ExprKind::Name(aivi_hir::TermReference::resolved(
                test_path("value"),
                aivi_hir::TermResolution::Local(binding),
            )),
        })
        .expect("local expression allocation should fit");
    let add_one = module
        .push_item(aivi_hir::Item::Function(aivi_hir::FunctionItem {
            header: aivi_hir::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("addOne"),
            type_parameters: Vec::new(),
            context: Vec::new(),
            parameters: vec![aivi_hir::FunctionParameter {
                span: unit_span(),
                binding,
                annotation: Some(int_type),
            }],
            annotation: Some(int_type),
            body: local_expr,
        }))
        .expect("function allocation should fit");
    let head = module
        .alloc_expr(aivi_hir::Expr {
            span: unit_span(),
            kind: aivi_hir::ExprKind::Integer(aivi_hir::IntegerLiteral { raw: "1".into() }),
        })
        .expect("head allocation should fit");
    let callable_expr = module
        .alloc_expr(aivi_hir::Expr {
            span: unit_span(),
            kind: aivi_hir::ExprKind::Name(aivi_hir::TermReference::resolved(
                test_path("addOne"),
                aivi_hir::TermResolution::Item(add_one),
            )),
        })
        .expect("callable expression allocation should fit");
    let replacement_expr = module
        .alloc_expr(aivi_hir::Expr {
            span: unit_span(),
            kind: aivi_hir::ExprKind::Text(aivi_hir::TextLiteral {
                segments: vec![aivi_hir::TextSegment::Text(aivi_hir::TextFragment {
                    raw: "done".into(),
                    span: unit_span(),
                })],
            }),
        })
        .expect("replacement expression allocation should fit");
    let pipe = module
        .alloc_expr(aivi_hir::Expr {
            span: unit_span(),
            kind: aivi_hir::ExprKind::Pipe(aivi_hir::PipeExpr {
                head,
                stages: aivi_hir::NonEmpty::new(
                    aivi_hir::PipeStage {
                        span: unit_span(),
                        subject_memo: None,
                        result_memo: None,
                        kind: aivi_hir::PipeStageKind::Transform {
                            expr: callable_expr,
                        },
                    },
                    vec![aivi_hir::PipeStage {
                        span: unit_span(),
                        subject_memo: None,
                        result_memo: None,
                        kind: aivi_hir::PipeStageKind::Transform {
                            expr: replacement_expr,
                        },
                    }],
                ),
                result_block_desugaring: false,
            }),
        })
        .expect("pipe allocation should fit");
    let _final_label = module
        .push_item(aivi_hir::Item::Value(aivi_hir::ValueItem {
            header: aivi_hir::ItemHeader {
                span: unit_span(),
                decorators: Vec::new(),
            },
            name: test_name("finalLabel"),
            annotation: Some(text_type),
            body: pipe,
        }))
        .expect("value allocation should fit");

    let core = lower_module(&module).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let final_label = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "finalLabel")
        .map(|(id, _)| id)
        .expect("expected finalLabel value item");
    let body = core.items()[final_label]
        .body
        .expect("finalLabel should carry a lowered body");
    let crate::ExprKind::Pipe(pipe) = &core.exprs()[body].kind else {
        panic!("finalLabel should lower to a pipe expression");
    };
    assert_eq!(pipe.stages.len(), 2);
    let crate::PipeStageKind::Transform {
        mode: first_mode,
        expr: first_expr,
    } = &pipe.stages[0].kind
    else {
        panic!("first stage should remain a transform");
    };
    assert_eq!(*first_mode, PipeTransformMode::Apply);
    assert!(matches!(
        core.exprs()[*first_expr].kind,
        crate::ExprKind::Apply { .. }
    ));

    let crate::PipeStageKind::Transform {
        mode: second_mode,
        expr: second_expr,
    } = &pipe.stages[1].kind
    else {
        panic!("second stage should remain a transform");
    };
    assert_eq!(*second_mode, PipeTransformMode::Replace);
    assert!(matches!(
        core.exprs()[*second_expr].kind,
        crate::ExprKind::Text(_)
    ));
}

#[test]
fn lowers_nonfinal_generic_pipe_transforms_through_transparent_alias_parameters() {
    let lowered = lower_text(
        "typed-core-generic-pipe-alias.aivi",
        r#"
type Box A = {
    value: A
}

fun unwrapBox:A = box:(Box A)=>    box
    ||> { value } -> value

fun addOne:Int = value:Int=>    value + 1

value out : Int =
    { value: 1 }
      |> unwrapBox
      |> addOne
"#,
    );
    assert!(
        !lowered.has_errors(),
        "generic alias pipe example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");
}

#[test]
fn lowers_source_and_decode_programs_into_core_ir() {
    let lowered = lower_text(
        "typed-core-source-decode.aivi",
        r#"
domain Duration over Int = {
    parse : Int -> Result Text Duration
    value : Duration -> Int
}

@source custom.feed
signal timeout : Signal Duration
"#,
    );
    assert!(
        !lowered.has_errors(),
        "source/decode example should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let timeout = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "timeout")
        .map(|(id, _)| id)
        .expect("expected timeout signal item");
    let ItemKind::Signal(info) = &core.items()[timeout].kind else {
        panic!("timeout should remain a signal item");
    };
    let source = info
        .source
        .expect("timeout should carry a lowered source node");
    let decode = core.sources()[source]
        .decode
        .expect("source should carry a decode program");
    match &core.decode_programs()[decode].steps()[core.decode_programs()[decode].root] {
        DecodeStep::Domain { surface, .. } => {
            assert_eq!(surface.member_name.as_ref(), "parse");
            assert_eq!(surface.kind, crate::DomainDecodeSurfaceKind::FallibleResult);
        }
        other => panic!("expected domain decode root, found {other:?}"),
    }
}

#[test]
fn lowers_source_payload_values_into_typed_core_ir() {
    let lowered = lower_text(
        "typed-core-source-config.aivi",
        r#"
signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval = 5

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int
"#,
    );
    assert!(
        !lowered.has_errors(),
        "source config fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let users = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "users")
        .map(|(id, _)| id)
        .expect("expected users signal item");
    let ItemKind::Signal(info) = &core.items()[users].kind else {
        panic!("users should remain a signal item");
    };
    let source = &core.sources()[info
        .source
        .expect("users should carry a lowered source node")];
    assert_eq!(source.arguments.len(), 1);
    assert_eq!(source.options.len(), 3);
    assert!(matches!(
        core.exprs()[source.arguments[0].runtime_expr].kind,
        crate::ExprKind::Text(_)
    ));
    assert_eq!(source.options[0].option_name.as_ref(), "refreshOn");
    assert_eq!(source.options[1].option_name.as_ref(), "activeWhen");
    assert_eq!(source.options[2].option_name.as_ref(), "refreshEvery");
}

#[test]
fn lowers_value_and_function_bodies_into_typed_core_exprs() {
    let lowered = lower_text(
        "typed-core-general-exprs.aivi",
        r#"
value answer = 42

fun add:Int = x:Int y:Int=>    x + y
"#,
    );
    assert!(
        !lowered.has_errors(),
        "general-expression fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let answer = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "answer")
        .map(|(id, _)| id)
        .expect("expected answer value item");
    let answer_body = core.items()[answer]
        .body
        .expect("answer should carry a lowered value body");
    assert!(matches!(
        core.exprs()[answer_body].kind,
        crate::ExprKind::Integer(_)
    ));

    let add = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "add")
        .map(|(id, _)| id)
        .expect("expected add function item");
    assert_eq!(core.items()[add].parameters.len(), 2);
    let add_body = core.items()[add]
        .body
        .expect("add should carry a lowered function body");
    assert!(matches!(
        core.exprs()[add_body].kind,
        crate::ExprKind::Binary {
            operator: aivi_hir::BinaryOperator::Add,
            ..
        }
    ));
}

#[test]
fn lowers_case_and_truthy_falsy_pipe_bodies() {
    let lowered = lower_fixture("milestone-1/valid/pipes/pipe_algebra.aivi");
    assert!(
        !lowered.has_errors(),
        "pipe algebra fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let status_label = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "statusLabel")
        .map(|(id, _)| id)
        .expect("expected statusLabel function item");
    let status_body = core.items()[status_label]
        .body
        .expect("statusLabel should carry a lowered body");
    let crate::ExprKind::Pipe(status_pipe) = &core.exprs()[status_body].kind else {
        panic!("statusLabel should lower to a pipe expression");
    };
    assert!(matches!(
        status_pipe.stages[0].kind,
        crate::PipeStageKind::Case { .. }
    ));

    let start_or_wait = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "startOrWait")
        .map(|(id, _)| id)
        .expect("expected startOrWait function item");
    let start_or_wait_body = core.items()[start_or_wait]
        .body
        .expect("startOrWait should carry a lowered body");
    let crate::ExprKind::Pipe(branch_pipe) = &core.exprs()[start_or_wait_body].kind else {
        panic!("startOrWait should lower to a pipe expression");
    };
    assert!(matches!(
        branch_pipe.stages[0].kind,
        crate::PipeStageKind::TruthyFalsy(_)
    ));
}

#[test]
fn lowers_recurrence_reports_into_pipe_nodes() {
    let lowered = lower_text(
        "typed-core-recurrence.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}

domain Retry over Int = {
    suffix times : Int = value => Retry value
}

fun step:Int = n:Int=>    n

@recur.timer 5sec
signal polled : Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
    );
    assert!(
        !lowered.has_errors(),
        "recurrence fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let polled = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "polled")
        .map(|(id, _)| id)
        .expect("expected polled signal item");
    let pipe = &core.pipes()[core.items()[polled].pipes[0]];
    let recurrence = pipe
        .recurrence
        .as_ref()
        .expect("expected recurrence attachment");
    assert!(recurrence.guards.is_empty());
    assert_eq!(recurrence.steps.len(), 1);
    assert!(recurrence.non_source_wakeup.is_some());
}

#[test]
fn lowers_recurrence_guards_into_pipe_nodes() {
    let lowered = lower_text(
        "typed-core-recurrence-guard.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}

type Cursor = {
    hasNext: Bool
}

fun keep:Cursor = cursor:Cursor=>    cursor

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
        "guarded recurrence should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let cursor = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "cursor")
        .map(|(id, _)| id)
        .expect("expected cursor signal item");
    let pipe = &core.pipes()[core.items()[cursor].pipes[0]];
    let recurrence = pipe
        .recurrence
        .as_ref()
        .expect("expected guarded recurrence attachment");
    assert_eq!(recurrence.guards.len(), 1);
    assert_eq!(recurrence.steps.len(), 1);
}

#[test]
fn rejects_blocked_hir_handoffs_instead_of_guessing() {
    let lowered = lower_fixture("milestone-2/invalid/gate-predicate-not-bool/main.aivi");
    let errors = lower_module(lowered.module()).expect_err("blocked gate should stop lowering");
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| matches!(error, LoweringError::BlockedGateStage { .. }))
    );
}

#[test]
fn lowers_workspace_imports_into_declaration_stubs() {
    let lowered = lower_fixture("milestone-2/valid/use-member-imports/main.aivi");
    assert!(
        !lowered.has_errors(),
        "workspace import fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("workspace imports should lower");
    let primary_provider = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "primaryProvider")
        .map(|(id, _)| id)
        .expect("expected primaryProvider value item");
    let primary_body = core.items()[primary_provider]
        .body
        .expect("primaryProvider should carry a lowered body");
    let crate::ExprKind::Reference(crate::Reference::Item(imported_item)) =
        &core.exprs()[primary_body].kind
    else {
        panic!("primaryProvider should lower to an imported item reference");
    };
    let imported = &core.items()[*imported_item];
    assert_eq!(imported.name.as_ref(), "http");
    assert!(matches!(imported.kind, ItemKind::Value));
    assert!(
        imported.body.is_none(),
        "imported declaration stubs should stay bodyless in typed-core"
    );
}

#[test]
fn workspace_constructor_imports_share_runtime_identity_with_workspace_functions() {
    let workspace_text = r#"
type View = {
    | MailView
    | TodoView

    type View -> Text
    viewTitle = .
     ||> MailView -> "Mail"
     ||> TodoView -> "Todo"
}

export (View, MailView, TodoView, viewTitle)
"#;
    let entry = lower_text_with_single_import(
        "main.aivi",
        r#"
use shared.nav (View, MailView, viewTitle)

value currentView : View = MailView
value headerTitle = viewTitle currentView
"#,
        vec!["shared", "nav"],
        "shared/nav.aivi",
        workspace_text,
    );
    assert!(
        !entry.has_errors(),
        "entry should lower cleanly before typed-core lowering: {:?}",
        entry.diagnostics()
    );
    let workspace = lower_text("shared/nav.aivi", workspace_text);
    assert!(
        !workspace.has_errors(),
        "workspace module should lower cleanly before typed-core lowering: {:?}",
        workspace.diagnostics()
    );

    let included_items = entry
        .module()
        .root_items()
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let core = lower_runtime_module_with_workspace(
        entry.module(),
        &[("shared.nav", workspace.module())],
        &included_items,
    )
    .expect("workspace runtime lowering should succeed");

    let mail_view = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "MailView")
        .map(|(id, _)| id)
        .expect("expected imported MailView stub");
    let mail_view_body = core.items()[mail_view]
        .body
        .expect("MailView stub should synthesize a constructor body");
    let constructor_origin = match &core.exprs()[mail_view_body].kind {
        crate::ExprKind::Reference(crate::Reference::SumConstructor(handle)) => handle.item,
        other => {
            panic!("expected MailView body to be a sum constructor reference, got {other:?}")
        }
    };

    let view_title = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "viewTitle")
        .map(|(id, _)| id)
        .expect("expected workspace viewTitle function");
    let view_title_body = core.items()[view_title]
        .body
        .expect("viewTitle should carry its compiled body");
    let crate::ExprKind::Pipe(pipe) = &core.exprs()[view_title_body].kind else {
        panic!("expected viewTitle body to lower into a pipe expression");
    };
    let stage = pipe
        .stages
        .first()
        .expect("viewTitle pipe should have a case stage");
    let crate::PipeStageKind::Case { arms } = &stage.kind else {
        panic!("expected viewTitle pipe stage to be a case expression");
    };
    let pattern_origin = arms
        .iter()
        .find_map(|arm| match &arm.pattern.kind {
            crate::PatternKind::Constructor { callee, .. } => match &callee.reference {
                crate::Reference::SumConstructor(handle) => Some(handle.item),
                _ => None,
            },
            _ => None,
        })
        .expect("expected viewTitle case arm constructor handle");

    assert_eq!(constructor_origin, pattern_origin);
}

#[test]
fn debug_decorator_injects_debug_pipe_stages() {
    let lowered = lower_text(
        "typed-core-debug.aivi",
        r#"
fun step:Int = n:Int=>    n + 1

@debug
value traced : Int =
    0
     |> step
     |> step
"#,
    );
    assert!(
        !lowered.has_errors(),
        "debug fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let traced = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "traced")
        .map(|(id, _)| id)
        .expect("expected traced value item");
    let traced_body = core.items()[traced]
        .body
        .expect("traced should carry a lowered body");
    let crate::ExprKind::Pipe(pipe) = &core.exprs()[traced_body].kind else {
        panic!("traced body should lower to a pipe expression");
    };
    assert_eq!(pipe.stages.len(), 5);
    assert!(matches!(
        &pipe.stages[0].kind,
        crate::expr::PipeStageKind::Debug { label } if label.as_ref() == "traced head"
    ));
    assert!(matches!(
        &pipe.stages[1].kind,
        crate::expr::PipeStageKind::Transform { .. }
    ));
    assert!(matches!(
        &pipe.stages[2].kind,
        crate::expr::PipeStageKind::Debug { label } if label.as_ref() == "traced stage 1"
    ));
    assert!(matches!(
        &pipe.stages[3].kind,
        crate::expr::PipeStageKind::Transform { .. }
    ));
    assert!(matches!(
        &pipe.stages[4].kind,
        crate::expr::PipeStageKind::Debug { label } if label.as_ref() == "traced stage 2"
    ));
}

#[test]
fn lowers_same_module_instance_member_calls_into_hidden_items() {
    let lowered = lower_text(
        "typed-core-same-module-instance-member.aivi",
        r#"
class Semigroup A = {
    append : A -> A -> A
}

type Blob = Blob Int

instance Semigroup Blob = {
    append left right =
        left
}

value combined:Blob =
    append (Blob 1) (Blob 2)
"#,
    );
    assert!(
        !lowered.has_errors(),
        "same-module instance example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let combined = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "combined")
        .map(|(id, _)| id)
        .expect("expected combined value item");
    let combined_body = core.items()[combined]
        .body
        .expect("combined should carry a lowered body");
    let crate::ExprKind::Apply { callee, .. } = &core.exprs()[combined_body].kind else {
        panic!("combined should lower to an apply expression");
    };
    let crate::ExprKind::Reference(crate::Reference::ExecutableEvidence(hidden_item)) =
        &core.exprs()[*callee].kind
    else {
        panic!("same-module class member should lower to authored executable evidence");
    };
    let hidden = &core.items()[*hidden_item];
    assert!(
        hidden.name.starts_with("instance#"),
        "expected hidden instance-member item name, found {}",
        hidden.name
    );
    assert!(
        hidden.body.is_some(),
        "hidden instance-member item should carry a lowered body"
    );
}

#[test]
fn lowers_higher_kinded_same_module_instance_member_calls_into_hidden_items() {
    let lowered = lower_text(
        "typed-core-higher-kinded-instance-member.aivi",
        r#"
class Applicative F = {
    pureInt : F Int
}

instance Applicative Option = {
    pureInt = Some 1
}

value chosen:Option Int =
    pureInt
"#,
    );
    assert!(
        !lowered.has_errors(),
        "higher-kinded instance example should lower to HIR: {:?}",
        lowered.diagnostics()
    );
    let hir_validation = aivi_hir::validate_module(
        lowered.module(),
        aivi_hir::ValidationMode::RequireResolvedNames,
    );
    assert!(
        hir_validation.is_ok(),
        "higher-kinded instance example should validate before typed-core lowering: {:?}",
        hir_validation.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let chosen = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "chosen")
        .map(|(id, _)| id)
        .expect("expected chosen value item");
    let chosen_body = core.items()[chosen]
        .body
        .expect("chosen should carry a lowered body");
    let crate::ExprKind::Reference(crate::Reference::ExecutableEvidence(hidden_item)) =
        &core.exprs()[chosen_body].kind
    else {
        panic!(
            "higher-kinded same-module class member should lower to authored executable evidence"
        );
    };
    let hidden = &core.items()[*hidden_item];
    assert!(
        hidden.name.starts_with("instance#"),
        "expected hidden higher-kinded instance-member item name, found {}",
        hidden.name
    );
    assert!(
        hidden.body.is_some(),
        "hidden higher-kinded instance-member item should carry a lowered body"
    );
}

#[test]
fn lowers_higher_kinded_class_instance_fixture_into_core_ir() {
    let lowered = lower_fixture("milestone-2/valid/higher-kinded-class-instances/main.aivi");
    assert!(
        !lowered.has_errors(),
        "higher-kinded class/instance fixture should lower to HIR: {:?}",
        lowered.diagnostics()
    );
    let hir_validation = aivi_hir::validate_module(
        lowered.module(),
        aivi_hir::ValidationMode::RequireResolvedNames,
    );
    assert!(
        hir_validation.is_ok(),
        "higher-kinded class/instance fixture should validate before typed-core lowering: {:?}",
        hir_validation.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let hidden_members = core
        .items()
        .iter()
        .filter(|(_, item)| item.name.starts_with("instance#") && item.body.is_some())
        .count();
    assert!(
        hidden_members >= 2,
        "expected typed-core lowering to synthesize hidden items for higher-kinded instance members"
    );
}

#[test]
fn lowers_builtin_authored_and_imported_higher_kinded_calls_through_executable_evidence_items() {
    let local = lower_text(
        "typed-core-uniform-hkt-evidence.aivi",
        r#"
type Box A = Box A

instance Functor Box = {
    map transform box =
        box
         ||> Box item -> Box (transform item)
}

instance Foldable Box = {
    reduce step seed box =
        box
         ||> Box item -> step seed item
}

type Int -> Int
func addOne = x =>
    x + 1

type Int -> Int -> Int
func add = total item =>
    total + item

value mappedBuiltin : Option Int =
    map addOne (Some 1)

value totalBuiltin : Int =
    reduce add 10 [1]

value mappedAuthored : Box Int =
    map addOne (Box 1)

value totalAuthored : Int =
    reduce add 10 (Box 1)
"#,
    );
    assert!(
        !local.has_errors(),
        "uniform higher-kinded evidence fixture should lower to HIR: {:?}",
        local.diagnostics()
    );
    let local_core = lower_module(local.module())
        .expect("local uniform evidence fixture should lower into typed core");

    let mapped_builtin = expect_builtin_evidence_item(
        &local_core,
        expect_value_apply_callee(&local_core, "mappedBuiltin"),
        BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Option),
    );
    assert!(
        local_core.items()[mapped_builtin]
            .name
            .starts_with("builtin-evidence#"),
        "builtin map call should lower through a materialized builtin evidence item"
    );

    let total_builtin = expect_builtin_evidence_item(
        &local_core,
        expect_value_apply_callee(&local_core, "totalBuiltin"),
        BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::List),
    );
    assert!(
        local_core.items()[total_builtin]
            .name
            .starts_with("builtin-evidence#"),
        "builtin reduce call should lower through a materialized builtin evidence item"
    );

    for name in ["mappedAuthored", "totalAuthored"] {
        let evidence_item = expect_executable_evidence_item(
            &local_core,
            expect_value_apply_callee(&local_core, name),
        );
        let hidden = &local_core.items()[evidence_item];
        assert!(
            hidden.name.starts_with("instance#"),
            "same-module authored `{name}` should lower through a hidden instance member, found {}",
            hidden.name
        );
        assert!(
            hidden.body.is_some(),
            "same-module authored `{name}` hidden evidence should carry a lowered body"
        );
    }

    let imported = lower_text_with_single_import(
        "main.aivi",
        r#"
use shared.box (
    Box
    one
)

type Int -> Int
func addOne = x =>
    x + 1

type Int -> Int -> Int
func add = total item =>
    total + item

value mappedImported : Box Int =
    map addOne one

value totalImported : Int =
    reduce add 10 one
"#,
        vec!["shared", "box"],
        "shared/box.aivi",
        r#"
type Box A = Box A

instance Functor Box = {
    map transform box =
        box
         ||> Box item -> Box (transform item)
}

instance Foldable Box = {
    reduce step seed box =
        box
         ||> Box item -> step seed item
}

value one : Box Int = Box 1

export (Box, one)
"#,
    );
    assert!(
        !imported.has_errors(),
        "imported higher-kinded evidence fixture should lower to HIR: {:?}",
        imported.diagnostics()
    );
    let imported_core = lower_module(imported.module())
        .expect("imported higher-kinded evidence fixture should lower into typed core");

    for name in ["mappedImported", "totalImported"] {
        let evidence_item = expect_executable_evidence_item(
            &imported_core,
            expect_value_apply_callee(&imported_core, name),
        );
        let hidden = &imported_core.items()[evidence_item];
        assert!(
            hidden.name.contains("instance#") || hidden.name.starts_with("__instance_"),
            "imported authored `{name}` should lower through synthetic instance evidence, found {}",
            hidden.name
        );
        assert!(
            hidden.body.is_some() || hidden.name.starts_with("__instance_"),
            "imported authored `{name}` evidence should either lower a body or preserve the imported instance binding"
        );
    }
}

#[test]
fn lowers_prelude_foldable_reduce_calls_into_builtin_intrinsics() {
    let lowered = lower_text(
        "typed-core-foldable-reduce.aivi",
        r#"
fun add:Int = acc:Int n:Int=>    acc + n

fun joinStep:Text = acc:Text s:Text=>    append acc s

value joined:Text =
    reduce joinStep "" ["hel", "lo"]

value total:Int =
    reduce add 10 [1, 2, 3]
"#,
    );
    assert!(
        !lowered.has_errors(),
        "reduce example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let joined = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "joined")
        .map(|(id, _)| id)
        .expect("expected joined value item");
    let joined_body = core.items()[joined]
        .body
        .expect("joined should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[joined_body].kind else {
        panic!("joined should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 3);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Reduce(BuiltinFoldableCarrier::List),
    );
}

#[test]
fn lowers_extended_typeclass_members_into_builtin_intrinsics() {
    let lowered = lower_text(
        "typed-core-extended-typeclasses.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

fun keepSmall:(Option Int) = n:Int=>    n < 3
     T|> Some n
     F|> None

fun punctuate:Text = s:Text=>    append s "!"

value okOne:Result Text Int =
    Ok 1

value ordered:Ordering =
    compare 1.0 2.0

value mapped:Result Text Int =
    bimap punctuate addOne okOne

value readyTask:Task Text Int =
    pure 3

value traversed:Option (List Int) =
    traverse keepSmall [1, 2]

value filtered:List Int =
    filterMap keepSmall [1, 3, 2]
"#,
    );
    assert!(
        !lowered.has_errors(),
        "extended typeclass example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");

    let ordered = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "ordered")
        .map(|(id, _)| id)
        .expect("expected ordered value item");
    let ordered_body = core.items()[ordered]
        .body
        .expect("ordered should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[ordered_body].kind else {
        panic!("ordered should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 2);
    let ordered_item = expect_executable_evidence_item(&core, *callee);
    let ordered_wrapper_body = core.items()[ordered_item]
        .body
        .expect("compare evidence item should carry a body");
    let crate::ExprKind::Apply {
        callee: compare_callee,
        arguments: compare_arguments,
    } = &core.exprs()[ordered_wrapper_body].kind
    else {
        panic!("compare evidence item should lower to a builtin wrapper call");
    };
    assert_eq!(
        compare_arguments.len(),
        core.items()[ordered_item].parameters.len()
    );
    let crate::ExprKind::Reference(Reference::BuiltinClassMember(
        BuiltinClassMemberIntrinsic::Compare { subject, .. },
    )) = &core.exprs()[*compare_callee].kind
    else {
        panic!("compare evidence item should call the builtin compare intrinsic");
    };
    assert_eq!(*subject, BuiltinOrdSubject::Float);

    let mapped = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "mapped")
        .map(|(id, _)| id)
        .expect("expected mapped value item");
    let mapped_body = core.items()[mapped]
        .body
        .expect("mapped should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[mapped_body].kind else {
        panic!("mapped should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 3);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Bimap(BuiltinBifunctorCarrier::Result),
    );

    let ready_task = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "readyTask")
        .map(|(id, _)| id)
        .expect("expected readyTask value item");
    let ready_task_body = core.items()[ready_task]
        .body
        .expect("readyTask should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[ready_task_body].kind else {
        panic!("readyTask should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 1);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Task),
    );

    let traversed = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "traversed")
        .map(|(id, _)| id)
        .expect("expected traversed value item");
    let traversed_body = core.items()[traversed]
        .body
        .expect("traversed should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[traversed_body].kind else {
        panic!("traversed should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Traverse {
            traversable: BuiltinTraversableCarrier::List,
            applicative: BuiltinApplicativeCarrier::Option,
        },
    );

    let filtered = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "filtered")
        .map(|(id, _)| id)
        .expect("expected filtered value item");
    let filtered_body = core.items()[filtered]
        .body
        .expect("filtered should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[filtered_body].kind else {
        panic!("filtered should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::FilterMap(BuiltinFilterableCarrier::List),
    );
}

#[test]
fn lowers_monad_and_chain_members_into_builtin_intrinsics() {
    let lowered = lower_text(
        "typed-core-monad-chain.aivi",
        r#"
fun nextOption:(Option Int) = n:Int=>    Some (n + 1)

fun nextResult:(Result Text Int) = n:Int=>    Ok (n + 1)

value okSeed:Result Text Int =
    Ok 4

value chainedOption:Option Int =
    chain nextOption (Some 2)

value joinedOption:Option Int =
    join (Some (Some 3))

value chainedResult:Result Text Int =
    chain nextResult okSeed

value joinedList:List Int =
    join [[1, 2], [3]]
"#,
    );
    assert!(
        !lowered.has_errors(),
        "monad/chain example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");

    let chained_option = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "chainedOption")
        .map(|(id, _)| id)
        .expect("expected chainedOption value item");
    let chained_option_body = core.items()[chained_option]
        .body
        .expect("chainedOption should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[chained_option_body].kind
    else {
        panic!("chainedOption should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Chain(BuiltinMonadCarrier::Option),
    );

    let joined_option = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "joinedOption")
        .map(|(id, _)| id)
        .expect("expected joinedOption value item");
    let joined_option_body = core.items()[joined_option]
        .body
        .expect("joinedOption should carry a lowered body");
    let crate::ExprKind::Apply { callee, arguments } = &core.exprs()[joined_option_body].kind
    else {
        panic!("joinedOption should lower to an apply expression");
    };
    assert_eq!(arguments.len(), 1);
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Join(BuiltinMonadCarrier::Option),
    );

    let chained_result = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "chainedResult")
        .map(|(id, _)| id)
        .expect("expected chainedResult value item");
    let chained_result_body = core.items()[chained_result]
        .body
        .expect("chainedResult should carry a lowered body");
    let crate::ExprKind::Apply { callee, .. } = &core.exprs()[chained_result_body].kind else {
        panic!("chainedResult should lower to an apply expression");
    };
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Chain(BuiltinMonadCarrier::Result),
    );

    let joined_list = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "joinedList")
        .map(|(id, _)| id)
        .expect("expected joinedList value item");
    let joined_list_body = core.items()[joined_list]
        .body
        .expect("joinedList should carry a lowered body");
    let crate::ExprKind::Apply { callee, .. } = &core.exprs()[joined_list_body].kind else {
        panic!("joinedList should lower to an apply expression");
    };
    expect_builtin_evidence_item(
        &core,
        *callee,
        BuiltinClassMemberIntrinsic::Join(BuiltinMonadCarrier::List),
    );
}

#[test]
fn lowers_task_executable_class_members_into_builtin_intrinsics() {
    let lowered = lower_text(
        "typed-core-task-typeclasses.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

fun liftTask:(Task Text Int) = n:Int=>    pure (n + 2)

value oneTask:Task Text Int =
    pure 1

value addOneTask:Task Text (Int -> Int) =
    pure addOne

value threeTask:Task Text Int =
    pure 3

value fourTask:Task Text Int =
    pure 4

value nestedTask:Task Text (Task Text Int) =
    pure fourTask

value mappedTask:Task Text Int =
    map addOne oneTask

value appliedTask:Task Text Int =
    apply addOneTask oneTask

value chainedTask:Task Text Int =
    chain liftTask threeTask

value joinedTask:Task Text Int =
    join nestedTask
"#,
    );
    assert!(
        !lowered.has_errors(),
        "task typeclass example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");

    for (name, expected) in [
        (
            "mappedTask",
            BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Task),
        ),
        (
            "appliedTask",
            BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Task),
        ),
        (
            "chainedTask",
            BuiltinClassMemberIntrinsic::Chain(BuiltinMonadCarrier::Task),
        ),
        (
            "joinedTask",
            BuiltinClassMemberIntrinsic::Join(BuiltinMonadCarrier::Task),
        ),
    ] {
        let item = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("expected {name} value item"));
        let body = core.items()[item]
            .body
            .unwrap_or_else(|| panic!("{name} should carry a lowered body"));
        let crate::ExprKind::Apply { callee, .. } = &core.exprs()[body].kind else {
            panic!("{name} should lower to an apply expression");
        };
        expect_builtin_evidence_item(&core, *callee, expected);
    }
}

#[test]
fn rejects_task_as_a_traverse_result_applicative() {
    let lowered = lower_text(
        "typed-core-traverse-task-result.aivi",
        r#"
fun liftTask:(Task Text Int) = n:Int=>    pure (n + 1)

value traversedTask:Task Text (List Int) =
    traverse liftTask [1, 2]
"#,
    );
    assert!(
        !lowered.has_errors(),
        "task traverse example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let errors = lower_module(lowered.module()).expect_err("task traverse should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::UnsupportedClassMemberDispatch { reason, .. }
            if *reason
                == "runtime lowering only supports traverse results in List, Option, Result, Validation, and Signal applicatives"
    )));
}

#[test]
fn completeness_check_reports_missing_general_expr_items() {
    let lowered = lower_text("typed-core-completeness.aivi", "value answer:Int = 42\n");
    assert!(
        !lowered.has_errors(),
        "completeness example should lower to HIR: {:?}",
        lowered.diagnostics()
    );
    let empty = aivi_hir::GeneralExprElaborationReport::new(Vec::new(), Vec::new(), Vec::new());
    let errors = validate_general_expr_report_completeness(lowered.module(), &empty, |_| true);
    assert!(errors.iter().any(|error| matches!(
            error,
            LoweringError::MissingGeneralExprElaboration { owner, .. }
                if matches!(&lowered.module().items()[*owner], aivi_hir::Item::Value(value) if value.name.text() == "answer")
        )));
}

#[test]
fn runtime_fragments_pull_same_module_instance_member_dependencies() {
    let lowered = lower_text(
        "typed-core-runtime-fragment-instance-member.aivi",
        r#"
class Semigroup A = {
    append : A -> A -> A
}

type Blob = Blob Int

instance Semigroup Blob = {
    append left right =
        left
}

value combined:Blob =
    append (Blob 1) (Blob 2)
"#,
    );
    assert!(
        !lowered.has_errors(),
        "runtime-fragment instance example should lower to HIR: {:?}",
        lowered.diagnostics()
    );

    let report = aivi_hir::elaborate_general_expressions(lowered.module());
    let combined = report
            .items()
            .iter()
            .find(|item| matches!(&lowered.module().items()[item.owner], aivi_hir::Item::Value(value) if value.name.text() == "combined"))
            .expect("expected combined elaboration");
    let aivi_hir::GeneralExprOutcome::Lowered(body) = &combined.outcome else {
        panic!("combined runtime fragment should elaborate");
    };
    let lowered_fragment = lower_runtime_fragment(
        lowered.module(),
        &RuntimeFragmentSpec {
            name: "combinedFragment".into(),
            owner: combined.owner,
            body_expr: combined.body_expr,
            parameters: combined.parameters.clone(),
            body: body.clone(),
        },
    )
    .expect("runtime fragment should lower with same-module instance dependency");
    assert!(
        lowered_fragment
            .module
            .items()
            .iter()
            .any(|(_, item)| item.name.starts_with("instance#") && item.body.is_some()),
        "runtime fragment should carry a lowered hidden instance-member dependency"
    );
}

#[test]
fn rejects_blocked_decode_programs() {
    let lowered = lower_text(
        "typed-core-blocked-decode.aivi",
        r#"
domain Duration over Int
    millis : Int -> Duration
    tryMillis : Int -> Result Text Duration
    value : Duration -> Int

@source custom.feed
signal timeout : Signal Duration
"#,
    );
    assert!(
        !lowered.has_errors(),
        "ambiguous decode example should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let errors =
        lower_module(lowered.module()).expect_err("ambiguous decode should block lowering");
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| matches!(error, LoweringError::BlockedDecodeProgram { .. }))
    );
}

#[test]
fn validator_catches_broken_recurrence_closure() {
    let lowered = lower_text(
        "typed-core-recurrence.aivi",
        r#"
domain Duration over Int = {
    suffix sec : Int = value => Duration value
}

domain Retry over Int = {
    suffix times : Int = value => Retry value
}

fun step:Int = n:Int=>    n

@recur.timer 5sec
signal polled : Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
    );
    let mut core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let pipe_id = core
        .pipes()
        .iter()
        .find(|(_, pipe)| pipe.recurrence.is_some())
        .map(|(id, _)| id)
        .expect("expected recurrence pipe");
    let pipe = core
        .pipes_mut()
        .get_mut(pipe_id)
        .expect("pipe should exist");
    let recurrence = pipe.recurrence.as_mut().expect("recurrence should exist");
    recurrence.steps[0].result_subject = Type::Primitive(aivi_hir::BuiltinType::Text);
    let errors =
        validate_module(&core).expect_err("manually broken recurrence should fail validation");
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| matches!(error, ValidationError::RecurrenceDoesNotClose { .. }))
    );
}

#[test]
fn validator_catches_broken_inline_case_stage_result_types() {
    let lowered = lower_fixture("milestone-1/valid/patterns/pattern_matching.aivi");
    let mut core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    let loaded_name = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "loadedName")
        .map(|(id, _)| id)
        .expect("expected loadedName function item");
    let body = core.items()[loaded_name]
        .body
        .expect("loadedName should carry a lowered body");
    let crate::ExprKind::Pipe(pipe) = &core.exprs()[body].kind else {
        panic!("loadedName should lower to a pipe expression");
    };
    let crate::PipeStageKind::Case { arms } = &pipe.stages[0].kind else {
        panic!("loadedName should start with a case stage");
    };
    let bad_arm = arms[0].body;
    core.exprs_mut()
        .get_mut(bad_arm)
        .expect("case arm body should exist")
        .ty = Type::Primitive(aivi_hir::BuiltinType::Int);
    let errors = validate_module(&core).expect_err("broken inline case stage should fail");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        ValidationError::InlinePipeCaseArmResultMismatch { .. }
    )));
}

#[test]
fn lowers_result_block_fixture_into_core_ir() {
    let lowered = lower_fixture("milestone-2/valid/result-block/main.aivi");
    assert!(
        !lowered.has_errors(),
        "result block fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let combined = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "combined")
        .map(|(id, _)| id)
        .expect("expected combined value item");
    let body = core.items()[combined]
        .body
        .expect("combined should carry a lowered body");
    let crate::ExprKind::Pipe(pipe) = &core.exprs()[body].kind else {
        panic!("combined should lower to a pipe expression");
    };
    let crate::PipeStageKind::Case { arms } = &pipe.stages[0].kind else {
        panic!("combined should start with a case stage");
    };
    assert_eq!(
        arms.len(),
        2,
        "result block bindings should lower into Ok/Err case arms"
    );
}

#[test]
fn lowers_result_blocks_with_ok_sources_and_nested_bool_tails() {
    let lowered = lower_text(
        "result-block-ok-sources.aivi",
        "value promoted: Result Text Bool =\n\
             \x20\x20\x20\x20result {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20number <- Ok 20\n\
             \x20\x20\x20\x20\x20\x20\x20\x20number > 0\n\
             \x20\x20\x20\x20}\n\
             \n\
             value nestedPromoted: Result Text Bool =\n\
             \x20\x20\x20\x20result {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20flag <- result {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20base <- Ok 20\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20base > 0\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20flag\n\
             \x20\x20\x20\x20}\n",
    );
    assert!(
        !lowered.has_errors(),
        "result block ok-source sample should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module())
        .expect("typed-core lowering should accept ok-source result blocks");
    validate_module(&core).expect("lowered core module should validate");
}

#[test]
fn lowers_applicative_cluster_fixture_into_core_ir() {
    let lowered = lower_fixture("milestone-2/valid/applicative-clusters/main.aivi");
    assert!(
        !lowered.has_errors(),
        "applicative cluster fixture should lower cleanly before typed-core lowering: {:?}",
        lowered.diagnostics()
    );

    let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
    validate_module(&core).expect("lowered core module should validate");

    let validated_user = core
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == "validatedUser")
        .map(|(id, _)| id)
        .expect("expected validatedUser signal item");
    let ItemKind::Signal(_) = &core.items()[validated_user].kind else {
        panic!("validatedUser should remain a signal item");
    };
    let body = core.items()[validated_user]
        .body
        .expect("validatedUser should carry a lowered body");
    let crate::ExprKind::Apply {
        callee: outer_callee,
        arguments: outer_arguments,
    } = &core.exprs()[body].kind
    else {
        panic!("validatedUser should lower to nested applicative apply expressions");
    };
    assert_eq!(outer_arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *outer_callee,
        BuiltinClassMemberIntrinsic::Apply(crate::BuiltinApplyCarrier::Signal),
    );

    let crate::ExprKind::Apply {
        callee: middle_callee,
        arguments: middle_arguments,
    } = &core.exprs()[outer_arguments[0]].kind
    else {
        panic!("validatedUser should keep nesting Signal apply intrinsics");
    };
    assert_eq!(middle_arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *middle_callee,
        BuiltinClassMemberIntrinsic::Apply(crate::BuiltinApplyCarrier::Signal),
    );

    let crate::ExprKind::Apply {
        callee: inner_callee,
        arguments: inner_arguments,
    } = &core.exprs()[middle_arguments[0]].kind
    else {
        panic!("validatedUser should use builtin Signal apply for the first member");
    };
    assert_eq!(inner_arguments.len(), 2);
    expect_builtin_evidence_item(
        &core,
        *inner_callee,
        BuiltinClassMemberIntrinsic::Apply(crate::BuiltinApplyCarrier::Signal),
    );

    let crate::ExprKind::Apply {
        callee: pure_callee,
        arguments: pure_arguments,
    } = &core.exprs()[inner_arguments[0]].kind
    else {
        panic!("validatedUser should seed the cluster with Applicative.pure");
    };
    assert_eq!(pure_arguments.len(), 1);
    expect_builtin_evidence_item(
        &core,
        *pure_callee,
        BuiltinClassMemberIntrinsic::Pure(BuiltinApplicativeCarrier::Signal),
    );
}
