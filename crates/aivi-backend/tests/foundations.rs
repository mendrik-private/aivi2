use std::{collections::BTreeMap, fs, path::PathBuf};

use aivi_backend::{
    BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinClassMemberIntrinsic, BuiltinFoldableCarrier, BuiltinFunctorCarrier, CodegenError,
    DecodeStepKind, DomainDecodeSurfaceKind, EvaluationError, GateStage as BackendGateStage,
    InlinePipeConstructor, InlinePipePatternKind, InlinePipeStageKind, ItemKind as BackendItemKind,
    KernelEvaluator, KernelExprKind, LayoutKind, LoweringError, NonSourceWakeupCause,
    RecurrenceTarget, RuntimeBigInt, RuntimeDecimal, RuntimeFloat, RuntimeSumValue, RuntimeValue,
    SourceProvider, StageKind as BackendStageKind, ValidationError, compile_program,
    lower_module as lower_backend_module, validate_program,
};
use aivi_base::{SourceDatabase, SourceSpan};
use aivi_core::{
    Expr as CoreExpr, ExprKind as CoreExprKind, GateStage as CoreGateStage, Item as CoreItem,
    ItemKind as CoreItemKind, Module as CoreModule, Pipe as CorePipe, PipeOrigin as CorePipeOrigin,
    ProjectionBase as CoreProjectionBase, RecordField as CoreRecordField,
    Reference as CoreReference, Stage as CoreStage, StageKind as CoreStageKind, Type as CoreType,
    lower_module as lower_core_module, validate_module as validate_core_module,
};
use aivi_hir::{
    BigIntLiteral, BinaryOperator as HirBinaryOperator, BindingId as HirBindingId,
    BuiltinTerm as HirBuiltinTerm, BuiltinType, DecimalLiteral, ExprId as HirExprId, FloatLiteral,
    IntegerLiteral, ItemId as HirItemId,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::RootDatabase;
use aivi_syntax::parse_module;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

fn lower_text(path: &str, text: &str) -> aivi_backend::Program {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "backend test input should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let hir = aivi_hir::lower_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "backend test input should lower to HIR: {:?}",
        hir.diagnostics()
    );
    let core = lower_core_module(hir.module()).expect("HIR should lower into typed core");
    validate_core_module(&core).expect("typed core should validate before backend lowering");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate before backend lowering");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");
    backend
}

fn lower_fixture(path: &str) -> aivi_backend::Program {
    let text = fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
    lower_text(path, &text)
}

fn lower_workspace_text(path: &str, text: &str) -> aivi_backend::Program {
    let db = RootDatabase::new();
    let absolute = fixture_root().join(path);
    let file = db.open_file(&absolute, text);
    let lowered = aivi_query::hir_module(&db, file);
    assert!(
        lowered.hir_diagnostics().is_empty(),
        "workspace fixture should lower to HIR: {:?}",
        lowered.hir_diagnostics()
    );
    let core =
        lower_core_module(lowered.module()).expect("workspace HIR should lower into typed core");
    validate_core_module(&core).expect("workspace typed core should validate");
    let lambda = lower_lambda_module(&core).expect("workspace lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("workspace lambda module should validate");
    let backend = lower_backend_module(&lambda).expect("workspace backend lowering should succeed");
    validate_program(&backend).expect("workspace backend program should validate");
    backend
}

fn find_item(program: &aivi_backend::Program, name: &str) -> aivi_backend::ItemId {
    program
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("expected backend item `{name}`"))
}

fn first_pipeline(
    program: &aivi_backend::Program,
    item: aivi_backend::ItemId,
) -> aivi_backend::PipelineId {
    program.items()[item].pipelines[0]
}

fn clif_pointer_ty() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "i64"
    } else {
        "i32"
    }
}

fn manual_core_gate(when_true: CoreExprKind) -> CoreModule {
    manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Int),
        move |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: when_true,
                })
                .expect("expression allocation should fit")
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "0".into() }),
                })
                .expect("expression allocation should fit")
        },
    )
}

fn manual_core_gate_stage<F, G>(
    input_subject: CoreType,
    result_subject: CoreType,
    when_true: F,
    when_false: G,
) -> CoreModule
where
    F: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
    G: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
{
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "captured".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("item allocation should fit");
    let when_true = when_true(&mut module, span);
    let when_false = when_false(&mut module, span);
    let pipe_id = module
        .pipes_mut()
        .alloc(CorePipe {
            owner: item_id,
            origin: CorePipeOrigin {
                owner: HirItemId::from_raw(0),
                pipe_expr: HirExprId::from_raw(0),
                span,
            },
            stages: Vec::new(),
            recurrence: None,
        })
        .expect("pipe allocation should fit");
    let stage_id = module
        .stages_mut()
        .alloc(CoreStage {
            pipe: pipe_id,
            index: 0,
            span,
            input_subject,
            result_subject,
            kind: CoreStageKind::Gate(CoreGateStage::Ordinary {
                when_true,
                when_false,
            }),
        })
        .expect("stage allocation should fit");
    module
        .pipes_mut()
        .get_mut(pipe_id)
        .expect("pipe should exist")
        .stages
        .push(stage_id);
    module
        .items_mut()
        .get_mut(item_id)
        .expect("item should exist")
        .pipes
        .push(pipe_id);
    module
}

#[test]
fn lowers_gate_fixture_into_backend_ir() {
    let backend = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
    let maybe_active = find_item(&backend, "maybeActive");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, maybe_active)];
    let stage = &pipeline.stages[0];
    let BackendStageKind::Gate(BackendGateStage::Ordinary {
        when_true,
        when_false,
    }) = &stage.kind
    else {
        panic!("expected gate stage in maybeActive pipeline");
    };
    assert_eq!(
        backend.kernels()[*when_true].input_subject,
        Some(stage.input_layout)
    );
    assert_eq!(backend.kernels()[*when_false].input_subject, None);
    let pretty = backend.pretty();
    assert!(pretty.contains("runtime-kernel-v1"));
    assert!(pretty.contains("gate-false"));
}

#[test]
fn lowers_source_decode_into_backend_plans() {
    let backend = lower_text(
        "backend-source-decode.aivi",
        r#"
domain Duration over Int
    parse : Int -> Result Text Duration
    value : Duration -> Int

@source custom.feed
sig timeout : Signal Duration
"#,
    );

    let timeout = find_item(&backend, "timeout");
    let BackendItemKind::Signal(signal) = &backend.items()[timeout].kind else {
        panic!("timeout should remain a signal item");
    };
    let source = &backend.sources()[signal.source.expect("timeout should carry a source")];
    assert!(
        matches!(source.provider, SourceProvider::Custom(ref key) if key.as_ref() == "custom.feed")
    );
    let decode = &backend.decode_plans()[source.decode.expect("source should carry a decode plan")];
    let root = &decode.steps()[decode.root];
    match &root.kind {
        DecodeStepKind::Domain { surface, .. } => {
            assert_eq!(surface.member_name.as_ref(), "parse");
            assert_eq!(surface.kind, DomainDecodeSurfaceKind::FallibleResult);
        }
        other => panic!("expected domain decode root, found {other:?}"),
    }
    assert!(matches!(
        backend.layouts()[root.layout].kind,
        LayoutKind::AnonymousDomain { .. }
    ));
}

#[test]
fn evaluates_multiplicative_builtin_arithmetic_with_precedence() {
    let backend = lower_text(
        "backend-multiplicative-builtins.aivi",
        "val total:Int = 2 + 3 * 4 - 8 / 2 + 14 % 4\n",
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "total"), &BTreeMap::new())
            .expect("multiplicative builtin arithmetic should evaluate"),
        RuntimeValue::Int(12)
    );
}

#[test]
fn division_by_zero_reports_backend_evaluation_error() {
    let backend = lower_text("backend-division-by-zero.aivi", "val broken:Int = 1 / 0\n");

    let mut evaluator = KernelEvaluator::new(&backend);
    assert!(matches!(
        evaluator.evaluate_item(find_item(&backend, "broken"), &BTreeMap::new()),
        Err(EvaluationError::InvalidBinaryArithmetic {
            reason: "division by zero",
            ..
        })
    ));
}

#[test]
fn evaluates_noninteger_literal_item_bodies_from_source() {
    let backend = lower_fixture("milestone-2/valid/noninteger-literals/main.aivi");
    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "pi"), &globals)
            .expect("Float literal item should evaluate"),
        RuntimeValue::Float(RuntimeFloat::parse_literal("3.14").expect("literal should parse"))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "amount"), &globals)
            .expect("Decimal literal item should evaluate"),
        RuntimeValue::Decimal(
            RuntimeDecimal::parse_literal("19.25d").expect("literal should parse")
        )
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "whole"), &globals)
            .expect("whole-number Decimal literal item should evaluate"),
        RuntimeValue::Decimal(RuntimeDecimal::parse_literal("19d").expect("literal should parse"))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "count"), &globals)
            .expect("BigInt literal item should evaluate"),
        RuntimeValue::BigInt(RuntimeBigInt::parse_literal("123n").expect("literal should parse"))
    );
}

#[test]
fn lowers_source_config_values_into_backend_kernels() {
    let backend = lower_text(
        "backend-source-config.aivi",
        r#"
sig apiHost = "https://api.example.com"
sig refresh = 0
sig enabled = True
sig pollInterval = 5

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
sig users : Signal Int
"#,
    );

    let users = find_item(&backend, "users");
    let BackendItemKind::Signal(signal) = &backend.items()[users].kind else {
        panic!("users should remain a signal item");
    };
    let source = &backend.sources()[signal.source.expect("users should carry a source")];
    assert_eq!(source.arguments.len(), 1);
    assert_eq!(source.options.len(), 3);

    let argument_kernel = &backend.kernels()[source.arguments[0].kernel];
    assert!(argument_kernel.input_subject.is_none());
    assert!(matches!(
        argument_kernel.origin.kind,
        aivi_backend::KernelOriginKind::SourceArgument { .. }
    ));
    assert!(matches!(
        argument_kernel.exprs()[argument_kernel.root].kind,
        KernelExprKind::Text(_)
    ));

    assert_eq!(source.options[0].option_name.as_ref(), "refreshOn");
    assert_eq!(source.options[1].option_name.as_ref(), "activeWhen");
    assert_eq!(source.options[2].option_name.as_ref(), "refreshEvery");
    for option in &source.options {
        let kernel = &backend.kernels()[option.kernel];
        assert!(kernel.input_subject.is_none());
        assert!(matches!(
            kernel.origin.kind,
            aivi_backend::KernelOriginKind::SourceOption { .. }
        ));
    }
}

#[test]
fn lowers_item_body_kernels_into_backend_items() {
    let backend = lower_text(
        "backend-item-bodies.aivi",
        r#"
fun addOne:Int #value:Int =>
    value + 1

val answer =
    addOne 41
"#,
    );

    let add_one = find_item(&backend, "addOne");
    let item = &backend.items()[add_one];
    assert_eq!(item.parameters.len(), 1);
    let body = item
        .body
        .expect("function item should lower a backend body kernel");
    assert!(matches!(
        backend.kernels()[body].origin.kind,
        aivi_backend::KernelOriginKind::ItemBody { .. }
    ));

    let answer = find_item(&backend, "answer");
    assert!(
        backend.items()[answer].body.is_some(),
        "value items should also retain body kernels"
    );
}

#[test]
fn runtime_evaluates_item_bodies_and_source_kernels() {
    let backend = lower_text(
        "backend-runtime-values.aivi",
        r#"
sig apiHost = "https://api.example.com"
sig refresh = 0
sig enabled = True
sig pollInterval = 5

fun addOne:Int #value:Int =>
    value + 1

val answer =
    addOne 41

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: addOne pollInterval
}
sig users : Signal Int
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::from([
        (
            find_item(&backend, "apiHost"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Text(
                "https://api.example.com".into(),
            ))),
        ),
        (
            find_item(&backend, "refresh"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Int(0))),
        ),
        (
            find_item(&backend, "enabled"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true))),
        ),
        (
            find_item(&backend, "pollInterval"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Int(5))),
        ),
    ]);

    let answer = find_item(&backend, "answer");
    assert_eq!(
        evaluator
            .evaluate_item(answer, &globals)
            .expect("item body should evaluate"),
        RuntimeValue::Int(42)
    );

    let users = find_item(&backend, "users");
    let BackendItemKind::Signal(signal) = &backend.items()[users].kind else {
        panic!("users should remain a signal item");
    };
    let source = &backend.sources()[signal.source.expect("source should exist")];

    assert_eq!(
        evaluator
            .evaluate_kernel(source.arguments[0].kernel, None, &[], &globals)
            .expect("source argument kernel should evaluate"),
        RuntimeValue::Text("https://api.example.com/users".into())
    );

    let active_when = source
        .options
        .iter()
        .find(|option| option.option_name.as_ref() == "activeWhen")
        .expect("activeWhen option should exist");
    assert_eq!(
        evaluator
            .evaluate_kernel(active_when.kernel, None, &[], &globals)
            .expect("activeWhen option should evaluate"),
        RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true)))
    );

    let refresh_every = source
        .options
        .iter()
        .find(|option| option.option_name.as_ref() == "refreshEvery")
        .expect("refreshEvery option should exist");
    assert_eq!(
        evaluator
            .evaluate_kernel(refresh_every.kernel, None, &[], &globals)
            .expect("refreshEvery option should evaluate"),
        RuntimeValue::Int(6)
    );
}

#[test]
fn runtime_evaluates_builtin_overloaded_class_members() {
    let backend = lower_text(
        "backend-builtin-class-members.aivi",
        r#"
fun increment:Int #value:Int => value + 1

val joined:Text =
    append "hel" "lo"

val lifted:Option Int =
    map increment (Some 1)

val singleton:Option Int =
    pure 3

val none:List Int =
    empty
"#,
    );

    let joined = find_item(&backend, "joined");
    let lifted = find_item(&backend, "lifted");
    let singleton = find_item(&backend, "singleton");
    let none = find_item(&backend, "none");
    let none_kernel_id = backend.items()[none]
        .body
        .expect("none should carry a body");

    let joined_kernel = backend.kernels()[backend.items()[joined]
        .body
        .expect("joined should carry a body")]
    .clone();
    match &joined_kernel.exprs()[joined_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &joined_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Append(
                    BuiltinAppendCarrier::Text
                ))
            ));
        }
        other => panic!("expected append body to lower into an apply tree, found {other:?}"),
    }

    let lifted_kernel = backend.kernels()[backend.items()[lifted]
        .body
        .expect("lifted should carry a body")]
    .clone();
    match &lifted_kernel.exprs()[lifted_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &lifted_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Map(
                    BuiltinFunctorCarrier::Option
                ))
            ));
        }
        other => panic!("expected map body to lower into an apply tree, found {other:?}"),
    }

    let singleton_kernel = backend.kernels()[backend.items()[singleton]
        .body
        .expect("singleton should carry a body")]
    .clone();
    match &singleton_kernel.exprs()[singleton_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &singleton_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Pure(
                    BuiltinApplicativeCarrier::Option
                ))
            ));
        }
        other => panic!("expected pure body to lower into an apply tree, found {other:?}"),
    }

    assert!(matches!(
        &backend.kernels()[none_kernel_id].exprs()[backend.kernels()[none_kernel_id].root].kind,
        KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Empty(
            BuiltinAppendCarrier::List
        ))
    ));

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(joined, &globals)
            .expect("append should evaluate"),
        RuntimeValue::Text("hello".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(lifted, &globals)
            .expect("map should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(singleton, &globals)
            .expect("pure should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(none, &globals)
            .expect("empty should evaluate"),
        RuntimeValue::List(Vec::new())
    );
}

#[test]
fn runtime_evaluates_builtin_foldable_reduce_members() {
    let backend = lower_text(
        "backend-foldable-reduce.aivi",
        r#"
fun add:Int #acc:Int #value:Int =>
    acc + value

fun joinStep:Text #acc:Text #value:Text =>
    append acc value

val maybeInput:Option Int =
    Some 4

val noneInput:Option Int =
    None

val okInput:Result Text Int =
    Ok 5

val errInput:Result Text Int =
    Err "bad"

val validInput:Validation Text Int =
    Valid 6

val invalidInput:Validation Text Int =
    Invalid "missing"

val total:Int =
    reduce add 10 [1, 2, 3]

val joined:Text =
    reduce joinStep "" ["hel", "lo"]

val maybeTotal:Int =
    reduce add 10 maybeInput

val noneTotal:Int =
    reduce add 10 noneInput

val okTotal:Int =
    reduce add 10 okInput

val errTotal:Int =
    reduce add 10 errInput

val validTotal:Int =
    reduce add 10 validInput

val invalidTotal:Int =
    reduce add 10 invalidInput
"#,
    );

    let total = find_item(&backend, "total");
    let joined = find_item(&backend, "joined");
    let maybe_total = find_item(&backend, "maybeTotal");
    let none_total = find_item(&backend, "noneTotal");
    let ok_total = find_item(&backend, "okTotal");
    let err_total = find_item(&backend, "errTotal");
    let valid_total = find_item(&backend, "validTotal");
    let invalid_total = find_item(&backend, "invalidTotal");

    let total_kernel = backend.kernels()[backend.items()[total]
        .body
        .expect("total should carry a body")]
    .clone();
    match &total_kernel.exprs()[total_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 3);
            assert!(matches!(
                &total_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Reduce(
                    BuiltinFoldableCarrier::List
                ))
            ));
        }
        other => panic!("expected reduce body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(total, &globals)
            .expect("list reduce should evaluate"),
        RuntimeValue::Int(16)
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined, &globals)
            .expect("text reduce should evaluate"),
        RuntimeValue::Text("hello".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(maybe_total, &globals)
            .expect("option reduce should evaluate"),
        RuntimeValue::Int(14)
    );
    assert_eq!(
        evaluator
            .evaluate_item(none_total, &globals)
            .expect("empty option reduce should evaluate"),
        RuntimeValue::Int(10)
    );
    assert_eq!(
        evaluator
            .evaluate_item(ok_total, &globals)
            .expect("result reduce should evaluate"),
        RuntimeValue::Int(15)
    );
    assert_eq!(
        evaluator
            .evaluate_item(err_total, &globals)
            .expect("error result reduce should evaluate"),
        RuntimeValue::Int(10)
    );
    assert_eq!(
        evaluator
            .evaluate_item(valid_total, &globals)
            .expect("validation reduce should evaluate"),
        RuntimeValue::Int(16)
    );
    assert_eq!(
        evaluator
            .evaluate_item(invalid_total, &globals)
            .expect("invalid validation reduce should evaluate"),
        RuntimeValue::Int(10)
    );
}

#[test]
fn runtime_evaluates_validation_apply_through_backend_runtime() {
    let backend = lower_text(
        "backend-validation-apply.aivi",
        r#"
type Pair = Pair Text Text

fun pair:Pair #left:Text #right:Text =>
    Pair left right

val first:Validation Text Text =
    Valid "Ada"

val second:Validation Text Text =
    Valid "Lovelace"

val combined:Validation Text Pair =
    apply (map pair first) second
"#,
    );

    let combined = find_item(&backend, "combined");
    let combined_kernel = &backend.kernels()[backend.items()[combined]
        .body
        .expect("combined should carry a body")];
    assert!(
        combined_kernel.exprs().iter().any(|(_, expr)| {
            matches!(
                expr.kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Apply(
                    BuiltinApplyCarrier::Validation
                ))
            )
        }),
        "expected validation applicative clusters to lower through builtin Apply"
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(combined, &globals)
        .expect("validation apply should evaluate")
    {
        RuntimeValue::ValidationValid(value) => match *value {
            RuntimeValue::Sum(value) => {
                assert_eq!(value.type_name.as_ref(), "Pair");
                assert_eq!(value.variant_name.as_ref(), "Pair");
                assert_eq!(
                    value.fields,
                    vec![
                        RuntimeValue::Text("Ada".into()),
                        RuntimeValue::Text("Lovelace".into()),
                    ]
                );
            }
            other => panic!("expected Pair payload, found {other:?}"),
        },
        other => panic!("expected valid validation result, found {other:?}"),
    }
}

#[test]
fn workspace_imported_builtin_class_members_lower_through_backend_runtime() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use shared.types (
    Envelope
)

fun increment:Int #value:Int =>
    value + 1

val lifted:Envelope (Option Int) =
    map increment (Some 1)
"#,
    );

    let lifted = find_item(&backend, "lifted");
    let lifted_kernel_id = backend.items()[lifted]
        .body
        .expect("workspace lifted should lower a backend body kernel");
    let lifted_kernel = &backend.kernels()[lifted_kernel_id];
    match &lifted_kernel.exprs()[lifted_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &lifted_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Map(
                    BuiltinFunctorCarrier::Option
                ))
            ));
        }
        other => panic!("expected workspace map call to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(lifted, &globals)
            .expect("workspace map call should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
}

#[test]
fn same_module_class_member_calls_lower_through_backend_runtime() {
    let backend = lower_text(
        "backend-same-module-class-member.aivi",
        r#"
class Semigroup A
    append : A -> A -> A

type Blob = Blob Int

instance Semigroup Blob
    append left right =
        left

fun combine:Blob #left:Blob #right:Blob =>
    append left right

val combined:Blob =
    combine (Blob 1) (Blob 2)
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(find_item(&backend, "combined"), &globals)
        .expect("same-module class member should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Blob");
            assert_eq!(value.variant_name.as_ref(), "Blob");
            assert_eq!(value.fields, vec![RuntimeValue::Int(1)]);
        }
        other => panic!("expected Blob sum result, found {other:?}"),
    }
}

#[test]
fn evaluates_inline_case_pipe_with_pattern_binding_and_parameter_capture() {
    let backend = lower_text(
        "backend-inline-case-captures.aivi",
        r#"
val fallback = "guest"

fun greet:Text #prefix:Text #maybeUser:(Option Text) =>
    maybeUser
     ||> Some name => "{prefix}:{name}"
     ||> None => "{prefix}:{fallback}"

val present =
    greet "user" (Some "Ada")

val missing =
    greet "user" None
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "present"), &globals)
            .expect("inline case with captures should evaluate"),
        RuntimeValue::Text("user:Ada".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "missing"), &globals)
            .expect("inline case fallback should evaluate"),
        RuntimeValue::Text("user:guest".into())
    );
}

#[test]
fn evaluates_inline_truthy_falsy_item_bodies() {
    let backend = lower_text(
        "backend-inline-truthy-falsy.aivi",
        r#"
val ready = True
val start = "start"
val wait = "wait"

val branch =
    ready
     T|> start
     F|> wait
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "branch"), &BTreeMap::new())
            .expect("truthy/falsy item body should evaluate"),
        RuntimeValue::Text("start".into())
    );
}

#[test]
fn evaluates_signal_carried_inline_case_pipes_with_committed_snapshots_and_captures() {
    let backend = lower_text(
        "backend-signal-inline-case-captures.aivi",
        r#"
fun greetSelected:Signal Text #prefix:Text #fallback:Text =>
    selectedUser
     ||> Some name => "{prefix}:{name}"
     ||> None => "{prefix}:{fallback}"

sig selectedUser : Signal (Option Text)

sig greeting : Signal Text =
    greetSelected "user" "guest"
"#,
    );

    let selected_user = find_item(&backend, "selectedUser");
    let greeting = find_item(&backend, "greeting");

    let present_globals = BTreeMap::from([(
        selected_user,
        RuntimeValue::Signal(Box::new(RuntimeValue::OptionSome(Box::new(
            RuntimeValue::Text("Ada".into()),
        )))),
    )]);
    assert_eq!(
        KernelEvaluator::new(&backend)
            .evaluate_item(greeting, &present_globals)
            .expect("signal-carried inline case should evaluate"),
        RuntimeValue::Text("user:Ada".into())
    );

    let missing_globals = BTreeMap::from([(
        selected_user,
        RuntimeValue::Signal(Box::new(RuntimeValue::OptionNone)),
    )]);
    assert_eq!(
        KernelEvaluator::new(&backend)
            .evaluate_item(greeting, &missing_globals)
            .expect("signal-carried inline case fallback should evaluate"),
        RuntimeValue::Text("user:guest".into())
    );
}

#[test]
fn evaluates_signal_carried_inline_truthy_falsy_pipes_with_committed_snapshots() {
    let backend = lower_text(
        "backend-signal-inline-truthy-falsy.aivi",
        r#"
fun renderStatus:Signal Text #prefix:Text #readyText:Text #waitText:Text =>
    ready
     T|> "{prefix}:{readyText}"
     F|> "{prefix}:{waitText}"

sig ready : Signal Bool

sig status : Signal Text =
    renderStatus "state" "go" "wait"
"#,
    );

    let ready = find_item(&backend, "ready");
    let status = find_item(&backend, "status");

    let ready_globals = BTreeMap::from([(
        ready,
        RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true))),
    )]);
    assert_eq!(
        KernelEvaluator::new(&backend)
            .evaluate_item(status, &ready_globals)
            .expect("signal-carried truthy branch should evaluate"),
        RuntimeValue::Text("state:go".into())
    );

    let waiting_globals = BTreeMap::from([(
        ready,
        RuntimeValue::Signal(Box::new(RuntimeValue::Bool(false))),
    )]);
    assert_eq!(
        KernelEvaluator::new(&backend)
            .evaluate_item(status, &waiting_globals)
            .expect("signal-carried falsy branch should evaluate"),
        RuntimeValue::Text("state:wait".into())
    );
}

#[test]
fn evaluates_inline_case_pipes_with_same_module_sum_variants_and_multiple_fields() {
    let backend = lower_text(
        "backend-inline-case-same-module-sums.aivi",
        r#"
type Status =
  | Idle
  | Ready Text
  | Failed Text Text

sig current : Signal Status

fun render:Text #status:Status =>
    status
     ||> Idle => "idle"
     ||> Ready name => name
     ||> Failed code message => "{code}:{message}"

val idleLabel =
    render current

val readyLabel =
    render (Ready "Ada")

val failedLabel =
    render (Failed "503" "offline")
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let current = find_item(&backend, "current");
    let render = find_item(&backend, "render");
    let render_kernel = backend.items()[render]
        .body
        .expect("render should lower to a backend body");
    let render_root = backend.kernels()[render_kernel].root;
    let KernelExprKind::Pipe(render_pipe) =
        &backend.kernels()[render_kernel].exprs()[render_root].kind
    else {
        panic!("render should lower to a pipe expression");
    };
    let InlinePipeStageKind::Case { arms } = &render_pipe.stages[0].kind else {
        panic!("render should lower to an inline case stage");
    };
    let InlinePipePatternKind::Constructor {
        constructor: InlinePipeConstructor::Sum(idle_handle),
        arguments,
    } = &arms[0].pattern.kind
    else {
        panic!("first case arm should carry the Idle constructor handle");
    };
    assert!(
        arguments.is_empty(),
        "Idle should remain a zero-field constructor arm"
    );
    let idle_globals = BTreeMap::from([(
        current,
        RuntimeValue::Signal(Box::new(RuntimeValue::Sum(RuntimeSumValue {
            item: idle_handle.item,
            type_name: idle_handle.type_name.clone(),
            variant_name: idle_handle.variant_name.clone(),
            fields: Vec::new(),
        }))),
    )]);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "idleLabel"), &idle_globals)
            .expect("zero-field sum arm should evaluate"),
        RuntimeValue::Text("idle".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "readyLabel"), &globals)
            .expect("single-field sum arm should evaluate"),
        RuntimeValue::Text("Ada".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "failedLabel"), &globals)
            .expect("multi-field sum arm should evaluate"),
        RuntimeValue::Text("503:offline".into())
    );
}

#[test]
fn lowers_recurrence_targets_and_witnesses() {
    let backend = lower_text(
        "backend-recurrence.aivi",
        r#"
domain Duration over Int
    literal s : Int -> Duration

domain Retry over Int
    literal x : Int -> Retry

fun step:Int #value:Int =>
    value

@recur.timer 5s
sig polled : Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3x
val retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
    );

    let polled = find_item(&backend, "polled");
    let polled_recurrence = backend.pipelines()[first_pipeline(&backend, polled)]
        .recurrence
        .as_ref()
        .expect("polled should carry a recurrence plan");
    assert_eq!(polled_recurrence.target, RecurrenceTarget::Signal);
    assert_eq!(polled_recurrence.steps.len(), 1);
    assert!(matches!(
        polled_recurrence
            .non_source_wakeup
            .as_ref()
            .map(|w| w.cause),
        Some(NonSourceWakeupCause::ExplicitTimer)
    ));

    let retried = find_item(&backend, "retried");
    let retried_recurrence = backend.pipelines()[first_pipeline(&backend, retried)]
        .recurrence
        .as_ref()
        .expect("retried should carry a recurrence plan");
    assert_eq!(retried_recurrence.target, RecurrenceTarget::Task);
    assert!(matches!(
        retried_recurrence
            .non_source_wakeup
            .as_ref()
            .map(|w| w.cause),
        Some(NonSourceWakeupCause::ExplicitBackoff)
    ));
}

#[test]
fn lowers_domain_operators_into_backend_gate_kernels() {
    let backend = lower_text(
        "backend-domain-operators.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration
    (>) : Duration -> Duration -> Bool

type Window = {
    delay: Duration
}

sig windows : Signal Window = { delay: 10ms }

sig slowWindows : Signal Window =
    windows
     ?|> ((.delay + 5ms) > 12ms)
"#,
    );

    let slow_windows = find_item(&backend, "slowWindows");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, slow_windows)];
    let BackendStageKind::Gate(BackendGateStage::SignalFilter { predicate, .. }) =
        &pipeline.stages[0].kind
    else {
        panic!("expected signal-filter gate stage for slowWindows");
    };

    let predicate_kernel = &backend.kernels()[*predicate];
    match &predicate_kernel.exprs()[predicate_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            match &predicate_kernel.exprs()[*callee].kind {
                KernelExprKind::DomainMember(handle) => {
                    assert_eq!(handle.domain_name.as_ref(), "Duration");
                    assert_eq!(handle.member_name.as_ref(), ">");
                }
                other => panic!(
                    "expected explicit domain-member callee for outer comparison, found {other:?}"
                ),
            }
            match &predicate_kernel.exprs()[arguments[0]].kind {
                KernelExprKind::Apply { callee, arguments } => {
                    assert_eq!(arguments.len(), 2);
                    match &predicate_kernel.exprs()[*callee].kind {
                        KernelExprKind::DomainMember(handle) => {
                            assert_eq!(handle.domain_name.as_ref(), "Duration");
                            assert_eq!(handle.member_name.as_ref(), "+");
                        }
                        other => panic!(
                            "expected explicit domain-member callee for nested add, found {other:?}"
                        ),
                    }
                    assert!(matches!(
                        &predicate_kernel.exprs()[arguments[0]].kind,
                        KernelExprKind::Projection { .. }
                    ));
                    assert!(matches!(
                        &predicate_kernel.exprs()[arguments[1]].kind,
                        KernelExprKind::SuffixedInteger(_)
                    ));
                }
                other => panic!(
                    "expected outer comparison left operand to be a nested apply tree, found {other:?}"
                ),
            }
            assert!(matches!(
                &predicate_kernel.exprs()[arguments[1]].kind,
                KernelExprKind::SuffixedInteger(_)
            ));
        }
        other => panic!(
            "expected predicate kernel to lower into an explicit apply tree, found {other:?}"
        ),
    }
}

#[test]
fn lowering_makes_local_bindings_explicit_environment_slots() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::Local(
        HirBindingId::from_raw(7),
    )));
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    let item = find_item(&backend, "captured");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
    let BackendStageKind::Gate(BackendGateStage::Ordinary {
        when_true,
        when_false,
    }) = &pipeline.stages[0].kind
    else {
        panic!("expected ordinary gate stage");
    };
    assert_eq!(backend.kernels()[*when_true].environment.len(), 1);
    assert_eq!(backend.kernels()[*when_true].input_subject, None);
    assert!(backend.kernels()[*when_false].environment.is_empty());
}

#[test]
fn validator_catches_missing_kernel_input_subject() {
    let mut backend = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
    let maybe_active = find_item(&backend, "maybeActive");
    let pipeline_id = first_pipeline(&backend, maybe_active);
    let when_true = match &backend.pipelines()[pipeline_id].stages[0].kind {
        BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) => *when_true,
        other => panic!("expected ordinary gate stage, found {other:?}"),
    };
    backend
        .kernels_mut()
        .get_mut(when_true)
        .expect("gate kernel should exist")
        .input_subject = None;

    let errors =
        validate_program(&backend).expect_err("missing input subject should fail validation");
    assert!(errors.errors().iter().any(|error| {
        matches!(
            error,
            ValidationError::KernelMissingInputSubject { kernel, .. }
                | ValidationError::KernelConventionMismatch { kernel }
                if *kernel == when_true
        )
    }));
}

#[test]
fn lowering_rejects_unresolved_hir_item_references() {
    let core = manual_core_gate(CoreExprKind::Reference(CoreReference::HirItem(
        HirItemId::from_raw(99),
    )));
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    let errors =
        lower_backend_module(&lambda).expect_err("unresolved HIR item reference should fail");
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| matches!(error, LoweringError::UnresolvedItemReference { .. }))
    );
}

#[test]
fn cranelift_codegen_compiles_scalar_gate_kernels() {
    let core = manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Bool),
        |module, span| {
            let subject = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::AmbientSubject,
                })
                .expect("subject allocation should fit");
            let one = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
                })
                .expect("integer allocation should fit");
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Binary {
                        left: subject,
                        operator: HirBinaryOperator::GreaterThan,
                        right: one,
                    },
                })
                .expect("comparison allocation should fit")
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Reference(CoreReference::Builtin(HirBuiltinTerm::False)),
                })
                .expect("builtin allocation should fit")
        },
    );
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
    let BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) =
        &pipeline.stages[0].kind
    else {
        panic!("expected ordinary gate stage");
    };

    let compiled = compile_program(&backend).expect("Cranelift codegen should succeed");
    let artifact = compiled
        .kernel(*when_true)
        .expect("compiled program should retain per-kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.symbol.contains("gate_true"));
    assert!(artifact.clif.contains("icmp sgt"));
    assert!(artifact.clif.contains("(i64) -> i8"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_real_gate_carrier_kernels() {
    let backend = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");

    let maybe_active = find_item(&backend, "maybeActive");
    let (when_true, when_false) =
        match &backend.pipelines()[first_pipeline(&backend, maybe_active)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::Ordinary {
                when_true,
                when_false,
            }) => (*when_true, *when_false),
            other => panic!("expected ordinary gate stage, found {other:?}"),
        };
    let active_users = find_item(&backend, "activeUsers");
    let predicate =
        match &backend.pipelines()[first_pipeline(&backend, active_users)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::SignalFilter { predicate, .. }) => *predicate,
            other => panic!("expected signal-filter gate stage, found {other:?}"),
        };

    let compiled =
        compile_program(&backend).expect("record projection and Option carriers should compile");
    let ptr = clif_pointer_ty();

    let gate_true = compiled
        .kernel(when_true)
        .expect("gate-true artifact should exist");
    assert!(gate_true.code_size > 0);
    assert!(gate_true.clif.contains(&format!("({ptr}) -> {ptr}")));

    let gate_false = compiled
        .kernel(when_false)
        .expect("gate-false artifact should exist");
    assert!(gate_false.code_size > 0);
    assert!(gate_false.clif.contains(&format!("iconst.{ptr} 0")));

    let predicate_artifact = compiled
        .kernel(predicate)
        .expect("predicate artifact should exist");
    assert!(predicate_artifact.code_size > 0);
    assert!(predicate_artifact.clif.contains(&format!("({ptr}) -> i8")));
    assert!(predicate_artifact.clif.contains("load.i8"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_environment_slots() {
    let core = manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Int),
        |module, span| {
            let captured = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Reference(CoreReference::Local(HirBindingId::from_raw(7))),
                })
                .expect("capture allocation should fit");
            let one = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
                })
                .expect("integer allocation should fit");
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Binary {
                        left: captured,
                        operator: HirBinaryOperator::Add,
                        right: one,
                    },
                })
                .expect("add allocation should fit")
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "0".into() }),
                })
                .expect("integer allocation should fit")
        },
    );
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
    let BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) =
        &pipeline.stages[0].kind
    else {
        panic!("expected ordinary gate stage");
    };

    let compiled = compile_program(&backend).expect("Cranelift codegen should succeed");
    let artifact = compiled
        .kernel(*when_true)
        .expect("compiled program should retain per-kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains("iadd"));
    assert!(artifact.clif.contains("(i64) -> i64"));
}

#[test]
fn cranelift_codegen_compiles_noninteger_literal_gate_kernels() {
    fn compile_literal_gate(
        result_type: BuiltinType,
        when_true: CoreExprKind,
        when_false: CoreExprKind,
        expected_value: RuntimeValue,
        expected_signature: &str,
        expected_clif_fragment: Option<&str>,
    ) {
        let result_ty = CoreType::Primitive(result_type);
        let core = manual_core_gate_stage(
            CoreType::Primitive(BuiltinType::Bool),
            result_ty.clone(),
            {
                let result_ty = result_ty.clone();
                move |module, span| {
                    module
                        .exprs_mut()
                        .alloc(CoreExpr {
                            span,
                            ty: result_ty.clone(),
                            kind: when_true,
                        })
                        .expect("literal allocation should fit")
                }
            },
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: result_ty.clone(),
                        kind: when_false,
                    })
                    .expect("fallback literal allocation should fit")
            },
        );
        validate_core_module(&core).expect("manual core module should validate");
        let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
        validate_lambda_module(&lambda).expect("typed lambda should validate");
        let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
        validate_program(&backend).expect("backend program should validate");

        let item = find_item(&backend, "captured");
        let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
        let BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) =
            &pipeline.stages[0].kind
        else {
            panic!("expected ordinary gate stage");
        };

        let mut evaluator = KernelEvaluator::new(&backend);
        assert_eq!(
            evaluator
                .evaluate_kernel(*when_true, None, &[], &BTreeMap::new())
                .expect("literal kernel should evaluate"),
            expected_value
        );

        let compiled = compile_program(&backend).expect("literal gate kernels should compile");
        let artifact = compiled
            .kernel(*when_true)
            .expect("compiled program should retain per-kernel metadata");
        assert!(artifact.code_size > 0);
        assert!(artifact.clif.contains(expected_signature));
        if let Some(fragment) = expected_clif_fragment {
            assert!(artifact.clif.contains(fragment));
        }
        assert!(!compiled.object().is_empty());
    }

    compile_literal_gate(
        BuiltinType::Float,
        CoreExprKind::Float(FloatLiteral { raw: "3.14".into() }),
        CoreExprKind::Float(FloatLiteral { raw: "2.5".into() }),
        RuntimeValue::Float(RuntimeFloat::parse_literal("3.14").expect("literal should parse")),
        "() -> f64",
        Some("f64const"),
    );
    compile_literal_gate(
        BuiltinType::Decimal,
        CoreExprKind::Decimal(DecimalLiteral {
            raw: "19.25d".into(),
        }),
        CoreExprKind::Decimal(DecimalLiteral { raw: "7d".into() }),
        RuntimeValue::Decimal(
            RuntimeDecimal::parse_literal("19.25d").expect("literal should parse"),
        ),
        &format!("() -> {}", clif_pointer_ty()),
        Some("symbol_value"),
    );
    compile_literal_gate(
        BuiltinType::BigInt,
        CoreExprKind::BigInt(BigIntLiteral { raw: "123n".into() }),
        CoreExprKind::BigInt(BigIntLiteral { raw: "456n".into() }),
        RuntimeValue::BigInt(RuntimeBigInt::parse_literal("123n").expect("literal should parse")),
        &format!("() -> {}", clif_pointer_ty()),
        Some("symbol_value"),
    );
}

#[test]
fn cranelift_codegen_compiles_by_reference_environment_projection() {
    let captured_user = CoreType::Record(vec![
        CoreRecordField {
            name: "active".into(),
            ty: CoreType::Primitive(BuiltinType::Bool),
        },
        CoreRecordField {
            name: "email".into(),
            ty: CoreType::Primitive(BuiltinType::Text),
        },
    ]);
    let core = manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Bool),
        {
            let captured_user = captured_user.clone();
            move |module, span| {
                let captured = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: captured_user.clone(),
                        kind: CoreExprKind::Reference(CoreReference::Local(
                            HirBindingId::from_raw(7),
                        )),
                    })
                    .expect("capture allocation should fit");
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: CoreType::Primitive(BuiltinType::Bool),
                        kind: CoreExprKind::Projection {
                            base: CoreProjectionBase::Expr(captured),
                            path: vec!["active".into()],
                        },
                    })
                    .expect("projection allocation should fit")
            }
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Reference(CoreReference::Builtin(HirBuiltinTerm::False)),
                })
                .expect("builtin allocation should fit")
        },
    );
    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let pipeline = &backend.pipelines()[first_pipeline(&backend, item)];
    let BackendStageKind::Gate(BackendGateStage::Ordinary { when_true, .. }) =
        &pipeline.stages[0].kind
    else {
        panic!("expected ordinary gate stage");
    };
    assert_eq!(backend.kernels()[*when_true].environment.len(), 1);
    assert_eq!(backend.kernels()[*when_true].input_subject, None);

    let compiled =
        compile_program(&backend).expect("by-reference environment projection should compile");
    let artifact = compiled
        .kernel(*when_true)
        .expect("compiled program should retain per-kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact
            .clif
            .contains(&format!("({}) -> i8", clif_pointer_ty()))
    );
    assert!(artifact.clif.contains("load.i8"));
}

#[test]
fn cranelift_codegen_prevalidates_invalid_projection_paths() {
    let backend = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
    let active_users = find_item(&backend, "activeUsers");
    let predicate =
        match &backend.pipelines()[first_pipeline(&backend, active_users)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::SignalFilter { predicate, .. }) => *predicate,
            other => panic!("expected signal-filter gate stage, found {other:?}"),
        };
    let mut backend = backend;
    let kernel = backend
        .kernels_mut()
        .get_mut(predicate)
        .expect("predicate kernel should exist");
    let root = kernel.root;
    let expr = kernel
        .exprs_mut()
        .get_mut(root)
        .expect("predicate root should exist");
    let KernelExprKind::Projection { path, .. } = &mut expr.kind else {
        panic!("expected predicate root to stay a projection");
    };
    path[0] = "missing".into();

    let errors =
        compile_program(&backend).expect_err("invalid projection path should fail prevalidation");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { kernel, expr, detail }
            if *kernel == predicate && *expr == root && detail.contains("no field `.missing`")
    )));
}

#[test]
fn cranelift_codegen_rejects_domain_apply_until_domain_lowering_exists() {
    let backend = lower_text(
        "backend-domain-operators-codegen.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration
    (>) : Duration -> Duration -> Bool

type Window = {
    delay: Duration
}

sig windows : Signal Window = { delay: 10ms }

sig slowWindows : Signal Window =
    windows
     ?|> ((.delay + 5ms) > 12ms)
"#,
    );
    let errors =
        compile_program(&backend).expect_err("domain apply kernels should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { detail, .. }
            if detail.contains("record projection, pointer-niche Option carriers")
    )));
}
