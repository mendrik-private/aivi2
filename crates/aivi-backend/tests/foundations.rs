use std::{collections::BTreeMap, fs, path::PathBuf};

use aivi_backend::{
    AbiPassMode, BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier,
    BuiltinBifunctorCarrier, BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier,
    BuiltinFoldableCarrier, BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject,
    BuiltinTerm, BuiltinTraversableCarrier, CodegenError, DecodeStepKind, DomainDecodeSurfaceKind,
    EvaluationError, GateStage as BackendGateStage, InlinePipeConstructor, InlinePipePatternKind,
    InlinePipeStageKind, ItemKind as BackendItemKind, KernelEvaluator, KernelExprKind,
    KernelOriginKind, LayoutKind, LoweringError, NonSourceWakeupCause, ProjectionBase,
    RecurrenceTarget, RuntimeBigInt, RuntimeDbCommitPlan, RuntimeDbConnection, RuntimeDbQueryPlan,
    RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeDecimal, RuntimeFloat, RuntimeRecordField,
    RuntimeSumValue, RuntimeTaskPlan, RuntimeValue, SourceProvider, StageKind as BackendStageKind,
    SubjectRef, ValidationError, compile_program, lower_module as lower_backend_module,
    validate_program,
};
use aivi_base::{SourceDatabase, SourceSpan};
use aivi_core::{
    Expr as CoreExpr, ExprKind as CoreExprKind, GateStage as CoreGateStage, Item as CoreItem,
    ItemKind as CoreItemKind, ItemParameter as CoreItemParameter, Module as CoreModule,
    Pipe as CorePipe, PipeExpr as CoreInlinePipeExpr, PipeOrigin as CorePipeOrigin,
    PipeStage as CoreInlinePipeStage, PipeStageKind as CoreInlinePipeStageKind,
    ProjectionBase as CoreProjectionBase, RecordField as CoreRecordField,
    Reference as CoreReference, Stage as CoreStage, StageKind as CoreStageKind, Type as CoreType,
    lower_module as lower_core_module, validate_module as validate_core_module,
};
use aivi_hir::{
    BigIntLiteral, BinaryOperator as HirBinaryOperator, BindingId as HirBindingId,
    BuiltinTerm as HirBuiltinTerm, BuiltinType, DecimalLiteral, ExprId as HirExprId, FloatLiteral,
    IntegerLiteral, ItemId as HirItemId, PipeTransformMode, SourceProviderRef,
    SourceReplacementPolicy as HirSourceReplacementPolicy,
    SourceStaleWorkPolicy as HirSourceStaleWorkPolicy,
    SourceTeardownPolicy as HirSourceTeardownPolicy, TypeParameterId as HirTypeParameterId,
};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_query::RootDatabase;
use aivi_syntax::parse_module;
use aivi_typing::SourceCancellationPolicy as TypingSourceCancellationPolicy;

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

fn manual_core_signal_filter_stage<F>(
    input_subject: CoreType,
    result_subject: CoreType,
    payload_type: CoreType,
    predicate: F,
) -> CoreModule
where
    F: FnOnce(&mut CoreModule, SourceSpan) -> aivi_core::ExprId,
{
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "filtered".into(),
            kind: CoreItemKind::Signal(aivi_core::SignalInfo::default()),
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("signal item allocation should fit");
    let predicate = predicate(&mut module, span);
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
            kind: CoreStageKind::Gate(CoreGateStage::SignalFilter {
                payload_type,
                predicate,
                emits_negative_update: true,
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

fn manual_core_source_argument_signal(
    argument_ty: CoreType,
    argument_kind: CoreExprKind,
) -> CoreModule {
    let span = SourceSpan::default();
    let mut module = CoreModule::new();
    let item_id = module
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "watched".into(),
            kind: CoreItemKind::Signal(aivi_core::SignalInfo::default()),
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("signal item allocation should fit");
    let runtime_expr = module
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: argument_ty,
            kind: argument_kind,
        })
        .expect("source argument allocation should fit");
    let source_id = module
        .sources_mut()
        .alloc(aivi_core::SourceNode {
            owner: item_id,
            span,
            instance: aivi_core::SourceInstanceId::from_raw(0),
            provider: SourceProviderRef::Missing,
            teardown: HirSourceTeardownPolicy::DisposeOnOwnerTeardown,
            replacement: HirSourceReplacementPolicy::DisposeSupersededBeforePublish,
            arguments: vec![aivi_core::SourceArgumentValue {
                origin_expr: HirExprId::from_raw(0),
                runtime_expr,
            }],
            options: Vec::new(),
            reconfiguration_dependencies: Vec::new(),
            explicit_triggers: Vec::new(),
            active_when: None,
            cancellation: TypingSourceCancellationPolicy::ProviderManaged,
            stale_work: HirSourceStaleWorkPolicy::DropStalePublications,
            decode: None,
        })
        .expect("source allocation should fit");
    let CoreItemKind::Signal(info) = &mut module
        .items_mut()
        .get_mut(item_id)
        .expect("signal item should exist")
        .kind
    else {
        unreachable!("manual source helper should keep the owner item a signal");
    };
    info.source = Some(source_id);
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
    type Int -> Result Text Duration
    parse
    type Duration -> Int
    unwrap

@source custom.feed
signal timeout : Signal Duration
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
        "value total:Int = 2 + 3 * 4 - 8 / 2 + 14 % 4\n",
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
fn evaluates_surface_subject_placeholders_and_integer_ranges() {
    let backend = lower_text(
        "backend-surface-subject-and-ranges.aivi",
        r#"value ambientFinal:Int =
    1
     |> 2
     |> 4
     |> .

value span:List Int = 1..3
value bracketed:List Int = [1..3]
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "ambientFinal"), &globals)
            .expect("ambient subject placeholder should evaluate"),
        RuntimeValue::Int(4)
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "span"), &globals)
            .expect("range expression should evaluate"),
        RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "bracketed"), &globals)
            .expect("bracketed range expression should evaluate"),
        RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])
    );
}

#[test]
fn division_by_zero_reports_backend_evaluation_error() {
    let backend = lower_text(
        "backend-division-by-zero.aivi",
        "value broken:Int = 1 / 0\n",
    );

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
fun addOne:Int = n:Int=>    n + 1

value answer =
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
signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval = 5

fun addOne:Int = n:Int=>    n + 1

value answer =
    addOne 41

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: addOne pollInterval
}
signal users : Signal Int
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
fun increment:Int = n:Int=> n + 1
value joined:Text =
    append "hel" "lo"

value lifted:Option Int =
    map increment (Some 1)

value singleton:Option Int =
    pure 3

value readyTask:Task Text Int =
    pure 3

value none:List Int =
    empty
"#,
    );

    let joined = find_item(&backend, "joined");
    let lifted = find_item(&backend, "lifted");
    let singleton = find_item(&backend, "singleton");
    let ready_task = find_item(&backend, "readyTask");
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

    let ready_task_kernel = backend.kernels()[backend.items()[ready_task]
        .body
        .expect("readyTask should carry a body")]
    .clone();
    match &ready_task_kernel.exprs()[ready_task_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &ready_task_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Pure(
                    BuiltinApplicativeCarrier::Task
                ))
            ));
        }
        other => panic!("expected task pure body to lower into an apply tree, found {other:?}"),
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
            .evaluate_item(ready_task, &globals)
            .expect("task pure should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(3)),
        })
    );
    assert_eq!(
        evaluator
            .evaluate_item(none, &globals)
            .expect("empty should evaluate"),
        RuntimeValue::List(Vec::new())
    );
}

#[test]
fn runtime_evaluates_builtin_monad_members() {
    let backend = lower_text(
        "backend-builtin-monad-members.aivi",
        r#"
fun nextOption:(Option Int) = n:Int=>    Some (n + 1)

fun nextResult:(Result Text Int) = n:Int=>    Ok (n + 2)

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

    let chained_option = find_item(&backend, "chainedOption");
    let joined_option = find_item(&backend, "joinedOption");
    let chained_result = find_item(&backend, "chainedResult");
    let joined_list = find_item(&backend, "joinedList");

    let chained_option_kernel = backend.kernels()[backend.items()[chained_option]
        .body
        .expect("chainedOption should carry a body")]
    .clone();
    match &chained_option_kernel.exprs()[chained_option_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &chained_option_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Chain(
                    BuiltinMonadCarrier::Option
                ))
            ));
        }
        other => panic!("expected chain body to lower into an apply tree, found {other:?}"),
    }

    let joined_option_kernel = backend.kernels()[backend.items()[joined_option]
        .body
        .expect("joinedOption should carry a body")]
    .clone();
    match &joined_option_kernel.exprs()[joined_option_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &joined_option_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Join(
                    BuiltinMonadCarrier::Option
                ))
            ));
        }
        other => panic!("expected join body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(chained_option, &globals)
            .expect("option chain should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined_option, &globals)
            .expect("option join should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(chained_result, &globals)
            .expect("result chain should evaluate"),
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(6)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined_list, &globals)
            .expect("list join should evaluate"),
        RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])
    );
}

#[test]
fn runtime_evaluates_applicative_clusters() {
    let backend = lower_text(
        "backend-applicative-clusters.aivi",
        r#"
fun add:Int = left:Int right:Int=>    left + right

value combinedOption:Option Int =
 &|> Some 2
 &|> Some 3
  |> add

value tupledOption:Option (Int, Int) =
 &|> Some 2
 &|> Some 3
"#,
    );

    let combined_option = find_item(&backend, "combinedOption");
    let tupled_option = find_item(&backend, "tupledOption");

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(combined_option, &globals)
            .expect("cluster finalizer should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(5)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(tupled_option, &globals)
            .expect("implicit tuple cluster should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Tuple(vec![
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])))
    );
}

#[test]
fn runtime_evaluates_builtin_foldable_reduce_members() {
    let backend = lower_text(
        "backend-foldable-reduce.aivi",
        r#"
fun add:Int = acc:Int item:Int=>    acc + item

fun joinStep:Text = acc:Text item:Text=>    append acc item

value maybeInput:Option Int =
    Some 4

value noneInput:Option Int =
    None

value okInput:Result Text Int =
    Ok 5

value errInput:Result Text Int =
    Err "bad"

value validInput:Validation Text Int =
    Valid 6

value invalidInput:Validation Text Int =
    Invalid "missing"

value total:Int =
    reduce add 10 [1, 2, 3]

value joined:Text =
    reduce joinStep "" ["hel", "lo"]

value maybeTotal:Int =
    reduce add 10 maybeInput

value noneTotal:Int =
    reduce add 10 noneInput

value okTotal:Int =
    reduce add 10 okInput

value errTotal:Int =
    reduce add 10 errInput

value validTotal:Int =
    reduce add 10 validInput

value invalidTotal:Int =
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
fn runtime_evaluates_extended_builtin_typeclass_members() {
    let backend = lower_text(
        "backend-extended-typeclass-members.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

fun keepSmall:(Option Int) = n:Int=>    n < 3
     T|> Some n
     F|> None

fun punctuate:Text = s:Text=>    append s "!"

value okOne:Result Text Int =
    Ok 1

value errBad:Result Text Int =
    Err "bad"

value validOne:Validation Text Int =
    Valid 1

value invalidNo:Validation Text Int =
    Invalid "no"

value orderedFloat:Ordering =
    compare 1.0 2.0

value orderedBig:Ordering =
    compare 1n 2n

value mappedOk:Result Text Int =
    bimap punctuate addOne okOne

value mappedErr:Result Text Int =
    bimap punctuate addOne errBad

value mappedValid:Validation Text Int =
    bimap punctuate addOne validOne

value mappedInvalid:Validation Text Int =
    bimap punctuate addOne invalidNo

value traversedList:Option (List Int) =
    traverse keepSmall [1, 2]

value traversedSome:Option (Option Int) =
    traverse keepSmall (Some 1)

value traversedOk:Option (Result Text Int) =
    traverse keepSmall okOne

value traversedValid:Option (Validation Text Int) =
    traverse keepSmall validOne

value filteredList:List Int =
    filterMap keepSmall [1, 3, 2]

value filteredSome:Option Int =
    filterMap keepSmall (Some 2)

value filteredMissing:Option Int =
    filterMap keepSmall (Some 4)
"#,
    );

    let ordered_float = find_item(&backend, "orderedFloat");
    let mapped_ok = find_item(&backend, "mappedOk");
    let traversed_list = find_item(&backend, "traversedList");
    let filtered_list = find_item(&backend, "filteredList");

    let ordered_float_kernel = backend.kernels()[backend.items()[ordered_float]
        .body
        .expect("orderedFloat should carry a body")]
    .clone();
    match &ordered_float_kernel.exprs()[ordered_float_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &ordered_float_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Compare {
                    subject: BuiltinOrdSubject::Float,
                    ..
                })
            ));
        }
        other => panic!("expected compare body to lower into an apply tree, found {other:?}"),
    }

    let mapped_ok_kernel = backend.kernels()[backend.items()[mapped_ok]
        .body
        .expect("mappedOk should carry a body")]
    .clone();
    match &mapped_ok_kernel.exprs()[mapped_ok_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 3);
            assert!(matches!(
                &mapped_ok_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Bimap(
                    BuiltinBifunctorCarrier::Result
                ))
            ));
        }
        other => panic!("expected bimap body to lower into an apply tree, found {other:?}"),
    }

    let traversed_list_kernel = backend.kernels()[backend.items()[traversed_list]
        .body
        .expect("traversedList should carry a body")]
    .clone();
    match &traversed_list_kernel.exprs()[traversed_list_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &traversed_list_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::Traverse {
                    traversable: BuiltinTraversableCarrier::List,
                    applicative: BuiltinApplicativeCarrier::Option
                })
            ));
        }
        other => panic!("expected traverse body to lower into an apply tree, found {other:?}"),
    }

    let filtered_list_kernel = backend.kernels()[backend.items()[filtered_list]
        .body
        .expect("filteredList should carry a body")]
    .clone();
    match &filtered_list_kernel.exprs()[filtered_list_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &filtered_list_kernel.exprs()[*callee].kind,
                KernelExprKind::BuiltinClassMember(BuiltinClassMemberIntrinsic::FilterMap(
                    BuiltinFilterableCarrier::List
                ))
            ));
        }
        other => panic!("expected filterMap body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(ordered_float, &globals)
        .expect("float compare should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Ordering");
            assert_eq!(value.variant_name.as_ref(), "Less");
        }
        other => panic!("expected Ordering result, found {other:?}"),
    }
    match evaluator
        .evaluate_item(find_item(&backend, "orderedBig"), &globals)
        .expect("bigint compare should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Ordering");
            assert_eq!(value.variant_name.as_ref(), "Less");
        }
        other => panic!("expected Ordering result, found {other:?}"),
    }
    assert_eq!(
        evaluator
            .evaluate_item(mapped_ok, &globals)
            .expect("result bimap should evaluate"),
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedErr"), &globals)
            .expect("err bimap should evaluate"),
        RuntimeValue::ResultErr(Box::new(RuntimeValue::Text("bad!".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedValid"), &globals)
            .expect("validation bimap should evaluate"),
        RuntimeValue::ValidationValid(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedInvalid"), &globals)
            .expect("invalid bimap should evaluate"),
        RuntimeValue::ValidationInvalid(Box::new(RuntimeValue::Text("no!".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(traversed_list, &globals)
            .expect("list traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
        ])))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedSome"), &globals)
            .expect("option traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::OptionSome(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedOk"), &globals)
            .expect("result traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::ResultOk(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedValid"), &globals)
            .expect("validation traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::ValidationValid(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(filtered_list, &globals)
            .expect("list filterMap should evaluate"),
        RuntimeValue::List(vec![RuntimeValue::Int(1), RuntimeValue::Int(2)])
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "filteredSome"), &globals)
            .expect("option filterMap should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "filteredMissing"), &globals)
            .expect("missing option filterMap should evaluate"),
        RuntimeValue::OptionNone
    );
}

#[test]
fn runtime_evaluates_validation_apply_through_backend_runtime() {
    let backend = lower_text(
        "backend-validation-apply.aivi",
        r#"
type Pair = Pair Text Text

fun pair:Pair = left:Text right:Text=>    Pair left right

value first:Validation Text Text =
    Valid "Ada"

value second:Validation Text Text =
    Valid "Lovelace"

value combined:Validation Text Pair =
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

fun increment:Int = n:Int=>    n + 1

value lifted:Envelope (Option Int) =
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

fun combine:Blob = left:Blob right:Blob=>    append left right

value combined:Blob =
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
fn evaluates_inline_pipe_transforms_with_apply_and_replace_modes() {
    let span = SourceSpan::default();
    let int_type = CoreType::Primitive(BuiltinType::Int);
    let text_type = CoreType::Primitive(BuiltinType::Text);
    let add_one_type = CoreType::Arrow {
        parameter: Box::new(int_type.clone()),
        result: Box::new(int_type.clone()),
    };
    let mut core = CoreModule::new();
    let add_one_binding = HirBindingId::from_raw(0);
    let add_one = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "addOne".into(),
            kind: CoreItemKind::Function,
            parameters: vec![CoreItemParameter {
                binding: add_one_binding,
                span,
                name: "value".into(),
                ty: int_type.clone(),
            }],
            body: None,
            pipes: Vec::new(),
        })
        .expect("function allocation should fit");
    let add_one_local = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Reference(CoreReference::Local(add_one_binding)),
        })
        .expect("local reference allocation should fit");
    let add_one_one = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
        })
        .expect("integer allocation should fit");
    let add_one_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Binary {
                left: add_one_local,
                operator: HirBinaryOperator::Add,
                right: add_one_one,
            },
        })
        .expect("function body allocation should fit");
    core.items_mut()
        .get_mut(add_one)
        .expect("function item should exist")
        .body = Some(add_one_body);

    let replaced = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(1),
            span,
            name: "replaced".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("value allocation should fit");
    let replaced_head = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
        })
        .expect("head allocation should fit");
    let replaced_two = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "2".into() }),
        })
        .expect("replacement allocation should fit");
    let replaced_four = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "4".into() }),
        })
        .expect("replacement allocation should fit");
    let replaced_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                head: replaced_head,
                stages: vec![
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: int_type.clone(),
                        result_subject: int_type.clone(),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: replaced_two,
                        },
                    },
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: int_type.clone(),
                        result_subject: int_type.clone(),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: replaced_four,
                        },
                    },
                ],
            }),
        })
        .expect("pipe body allocation should fit");
    core.items_mut()
        .get_mut(replaced)
        .expect("replaced item should exist")
        .body = Some(replaced_body);

    let called = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(2),
            span,
            name: "called".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("value allocation should fit");
    let called_head = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
        })
        .expect("head allocation should fit");
    let called_callee = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: add_one_type.clone(),
            kind: CoreExprKind::Reference(CoreReference::Item(add_one)),
        })
        .expect("callee allocation should fit");
    let called_subject = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::AmbientSubject,
        })
        .expect("ambient subject allocation should fit");
    let called_apply = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Apply {
                callee: called_callee,
                arguments: vec![called_subject],
            },
        })
        .expect("apply allocation should fit");
    let called_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                head: called_head,
                stages: vec![CoreInlinePipeStage {
                    span,
                    subject_memo: None,
                    result_memo: None,
                    input_subject: int_type.clone(),
                    result_subject: int_type.clone(),
                    kind: CoreInlinePipeStageKind::Transform {
                        mode: PipeTransformMode::Apply,
                        expr: called_apply,
                    },
                }],
            }),
        })
        .expect("pipe body allocation should fit");
    core.items_mut()
        .get_mut(called)
        .expect("called item should exist")
        .body = Some(called_body);

    let final_label = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(3),
            span,
            name: "finalLabel".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("value allocation should fit");
    let final_head = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
        })
        .expect("head allocation should fit");
    let final_callee = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: add_one_type.clone(),
            kind: CoreExprKind::Reference(CoreReference::Item(add_one)),
        })
        .expect("callee allocation should fit");
    let final_subject = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::AmbientSubject,
        })
        .expect("ambient subject allocation should fit");
    let final_apply = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: int_type.clone(),
            kind: CoreExprKind::Apply {
                callee: final_callee,
                arguments: vec![final_subject],
            },
        })
        .expect("apply allocation should fit");
    let final_text = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: text_type.clone(),
            kind: CoreExprKind::Text(aivi_core::TextLiteral {
                segments: vec![aivi_core::TextSegment::Fragment {
                    raw: "done".into(),
                    span,
                }],
            }),
        })
        .expect("text allocation should fit");
    let final_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: text_type.clone(),
            kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                head: final_head,
                stages: vec![
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: int_type.clone(),
                        result_subject: int_type.clone(),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Apply,
                            expr: final_apply,
                        },
                    },
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: int_type.clone(),
                        result_subject: text_type.clone(),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: final_text,
                        },
                    },
                ],
            }),
        })
        .expect("pipe body allocation should fit");
    core.items_mut()
        .get_mut(final_label)
        .expect("finalLabel item should exist")
        .body = Some(final_body);

    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let replaced = find_item(&backend, "replaced");
    let called = find_item(&backend, "called");
    let final_label = find_item(&backend, "finalLabel");

    let replaced_kernel = &backend.kernels()[backend.items()[replaced]
        .body
        .expect("replaced should carry a body")];
    let KernelExprKind::Pipe(replaced_pipe) = &replaced_kernel.exprs()[replaced_kernel.root].kind
    else {
        panic!("replaced should lower to a pipe expression");
    };
    assert_eq!(replaced_pipe.stages.len(), 2);
    assert!(matches!(
        &replaced_pipe.stages[0].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Replace,
            ..
        }
    ));
    assert!(matches!(
        &replaced_pipe.stages[1].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Replace,
            ..
        }
    ));

    let called_kernel = &backend.kernels()[backend.items()[called]
        .body
        .expect("called should carry a body")];
    let KernelExprKind::Pipe(called_pipe) = &called_kernel.exprs()[called_kernel.root].kind else {
        panic!("called should lower to a pipe expression");
    };
    assert_eq!(called_pipe.stages.len(), 1);
    assert!(matches!(
        &called_pipe.stages[0].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Apply,
            ..
        }
    ));

    let final_label_kernel = &backend.kernels()[backend.items()[final_label]
        .body
        .expect("finalLabel should carry a body")];
    let KernelExprKind::Pipe(final_label_pipe) =
        &final_label_kernel.exprs()[final_label_kernel.root].kind
    else {
        panic!("finalLabel should lower to a pipe expression");
    };
    assert_eq!(final_label_pipe.stages.len(), 2);
    assert!(matches!(
        &final_label_pipe.stages[0].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Apply,
            ..
        }
    ));
    assert!(matches!(
        &final_label_pipe.stages[1].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Replace,
            ..
        }
    ));

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(replaced, &globals)
            .expect("replacement transforms should evaluate"),
        RuntimeValue::Int(4)
    );
    assert_eq!(
        evaluator
            .evaluate_item(called, &globals)
            .expect("callable transform should evaluate"),
        RuntimeValue::Int(2)
    );
    assert_eq!(
        evaluator
            .evaluate_item(final_label, &globals)
            .expect("mixed transform modes should evaluate"),
        RuntimeValue::Text("done".into())
    );
}

#[test]
fn lowers_and_evaluates_inline_pipe_memos_across_stages() {
    let backend = lower_text(
        "backend-pipe-memos.aivi",
        r#"
fun add1:Int = x:Int=>    x + 1

value demo = 1
  |> #before add1 #after
  |> before + after
"#,
    );

    let demo = find_item(&backend, "demo");
    let kernel = &backend.kernels()[backend.items()[demo]
        .body
        .expect("demo should carry a body")];
    let KernelExprKind::Pipe(pipe) = &kernel.exprs()[kernel.root].kind else {
        panic!("demo should lower to an inline pipe expression");
    };
    assert_eq!(pipe.stages.len(), 2);
    assert!(pipe.stages[0].subject_memo.is_some());
    assert!(pipe.stages[0].result_memo.is_some());
    assert_eq!(pipe.stages[1].subject_memo, None);
    assert_eq!(pipe.stages[1].result_memo, None);

    let result = KernelEvaluator::new(&backend)
        .evaluate_item(demo, &BTreeMap::new())
        .expect("pipe memos should evaluate through backend inline pipes");
    assert_eq!(result, RuntimeValue::Int(3));
}

#[test]
fn evaluates_replacement_transform_stage_with_ambient_subject_value() {
    let span = SourceSpan::default();
    let mut core = CoreModule::new();
    let item = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "ambientFinal".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("item allocation should fit");

    let one = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
        })
        .expect("head allocation should fit");
    let two = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "2".into() }),
        })
        .expect("replacement allocation should fit");
    let four = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Integer(IntegerLiteral { raw: "4".into() }),
        })
        .expect("replacement allocation should fit");
    let ambient = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::AmbientSubject,
        })
        .expect("ambient subject allocation should fit");
    let body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                head: one,
                stages: vec![
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: CoreType::Primitive(BuiltinType::Int),
                        result_subject: CoreType::Primitive(BuiltinType::Int),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: two,
                        },
                    },
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: CoreType::Primitive(BuiltinType::Int),
                        result_subject: CoreType::Primitive(BuiltinType::Int),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: four,
                        },
                    },
                    CoreInlinePipeStage {
                        span,
                        subject_memo: None,
                        result_memo: None,
                        input_subject: CoreType::Primitive(BuiltinType::Int),
                        result_subject: CoreType::Primitive(BuiltinType::Int),
                        kind: CoreInlinePipeStageKind::Transform {
                            mode: PipeTransformMode::Replace,
                            expr: ambient,
                        },
                    },
                ],
            }),
        })
        .expect("pipe body allocation should fit");
    core.items_mut()
        .get_mut(item)
        .expect("item should exist")
        .body = Some(body);

    validate_core_module(&core).expect("manual core module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let kernel = &backend.kernels()[backend.items()[find_item(&backend, "ambientFinal")]
        .body
        .expect("ambientFinal should carry a body")];
    let KernelExprKind::Pipe(pipe) = &kernel.exprs()[kernel.root].kind else {
        panic!("ambientFinal should lower to a pipe expression");
    };
    assert!(matches!(
        &pipe.stages[2].kind,
        InlinePipeStageKind::Transform {
            mode: PipeTransformMode::Replace,
            ..
        }
    ));

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "ambientFinal"), &BTreeMap::new())
            .expect("ambient replacement transform should evaluate"),
        RuntimeValue::Int(4)
    );
}

#[test]
fn evaluates_inline_case_pipe_with_pattern_binding_and_parameter_capture() {
    let backend = lower_text(
        "backend-inline-case-captures.aivi",
        r#"
value fallback = "guest"

fun greet:Text = prefix:Text maybeUser:(Option Text)=>    maybeUser
     ||> Some name -> "{prefix}:{name}"
     ||> None -> "{prefix}:{fallback}"

value present =
    greet "user" (Some "Ada")

value missing =
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
value ready = True
value start = "start"
value wait = "wait"

value branch =
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
fun greetSelected:Signal Text = prefix:Text fallback:Text=>    selectedUser
     ||> Some name -> "{prefix}:{name}"
     ||> None -> "{prefix}:{fallback}"

signal selectedUser : Signal (Option Text)

signal greeting : Signal Text =
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
fun renderStatus:Signal Text = prefix:Text readyText:Text waitText:Text=>    ready
     T|> "{prefix}:{readyText}"
     F|> "{prefix}:{waitText}"

signal ready : Signal Bool

signal status : Signal Text =
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

signal current : Signal Status

fun render:Text = status:Status=>    status
     ||> Idle -> "idle"
     ||> Ready name -> name
     ||> Failed code message -> "{code}:{message}"

value idleLabel =
    render current

value readyLabel =
    render (Ready "Ada")

value failedLabel =
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
    literal sec : Int -> Duration

domain Retry over Int
    literal times : Int -> Retry

fun step:Int = x:Int=>    x

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
fn retains_signal_fanout_map_and_join_kernels() {
    let backend = lower_fixture("milestone-2/valid/pipe-fanout-carriers/main.aivi");
    let live_emails = find_item(&backend, "liveEmails");
    let live_joined = find_item(&backend, "liveJoinedEmails");

    let live_emails_body = backend.items()[live_emails]
        .body
        .expect("signal fanout map should retain a body prefix for startup linking");
    let live_joined_body = backend.items()[live_joined]
        .body
        .expect("signal fanout join should retain a body prefix for startup linking");
    assert!(matches!(
        backend.kernels()[live_emails_body].origin.kind,
        KernelOriginKind::ItemBody { item } if item == live_emails
    ));
    assert!(matches!(
        backend.kernels()[live_joined_body].origin.kind,
        KernelOriginKind::ItemBody { item } if item == live_joined
    ));

    let pipeline = &backend.pipelines()[first_pipeline(&backend, live_joined)];
    let BackendStageKind::Fanout(fanout) = &pipeline.stages[0].kind else {
        panic!("expected signal fanout stage for liveJoinedEmails");
    };
    assert!(matches!(
        backend.kernels()[fanout.map].origin.kind,
        KernelOriginKind::FanoutMap { stage_index, .. } if stage_index == 0
    ));
    let join = fanout
        .join
        .as_ref()
        .expect("joined fanout should retain a join kernel");
    assert!(matches!(
        backend.kernels()[join.kernel].origin.kind,
        KernelOriginKind::FanoutJoin { stage_index, .. } if stage_index == join.stage_index
    ));
}

#[test]
fn lowers_domain_operators_into_backend_gate_kernels() {
    let backend = lower_text(
        "backend-domain-operators.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
    type Duration -> Duration -> Bool
    (>)

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
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
fn runtime_evaluates_domain_operator_items_and_structural_equality() {
    let backend = lower_fixture("milestone-2/valid/domain-operator-usage/main.aivi");
    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "total"), &globals)
            .expect("domain operator item should evaluate"),
        RuntimeValue::SuffixedInteger {
            raw: "15".into(),
            suffix: "ms".into(),
        }
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "raw"), &globals)
            .expect("domain value projection should evaluate"),
        RuntimeValue::Int(15)
    );

    let equality = lower_text(
        "backend-domain-equality-runtime.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)

value same : Bool = 10ms + 5ms == 15ms
value different : Bool = 10ms + 5ms != 12ms
"#,
    );
    let mut evaluator = KernelEvaluator::new(&equality);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&equality, "same"), &BTreeMap::new())
            .expect("domain equality should evaluate"),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&equality, "different"), &BTreeMap::new())
            .expect("domain inequality should evaluate"),
        RuntimeValue::Bool(true)
    );

    let parameterized =
        lower_fixture("milestone-2/valid/domain-operator-usage-parameterized/main.aivi");
    let mut evaluator = KernelEvaluator::new(&parameterized);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&parameterized, "total"), &BTreeMap::new())
            .expect("parameterized domain operator should evaluate"),
        RuntimeValue::Int(3)
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&parameterized, "raw"), &BTreeMap::new())
            .expect("parameterized domain value projection should evaluate"),
        RuntimeValue::Int(3)
    );
}

#[test]
fn runtime_evaluates_domain_member_resolution_fixture() {
    let backend = lower_text(
        "backend-domain-member-resolution-runtime.aivi",
        r#"
domain NonEmpty A over List A
    type A -> NonEmpty A
    singleton
    type NonEmpty A -> A
    head
    type NonEmpty A -> List A
    tail

value items : NonEmpty Int = singleton 1
value first : Int = head items
value rest : List Int = tail items
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "first"), &globals)
            .expect("list-backed domain members should evaluate"),
        RuntimeValue::Int(1)
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "rest"), &globals)
            .expect("tail domain member should evaluate"),
        RuntimeValue::List(Vec::new())
    );
}

#[test]
fn runtime_evaluates_inline_pipe_domain_member_calls() {
    let backend = lower_text(
        "backend-inline-pipe-domain-member-runtime.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    type Duration -> Int
    unwrap

value raw : Int =
    10ms
     |> unwrap
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "raw"), &BTreeMap::new())
            .expect("inline pipe domain member should evaluate"),
        RuntimeValue::Int(10)
    );
}

#[test]
fn runtime_evaluates_authored_domain_member_items() {
    let backend = lower_text(
        "backend-authored-domain-member-runtime.aivi",
        r#"
type Builder = Int -> Duration

domain Duration over Int
    type Builder
    make raw = raw
    type Duration -> Int
    unwrap duration = duration

value raw : Int = unwrap (make 10)
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "raw"), &BTreeMap::new())
            .expect("authored domain members should evaluate through hidden items"),
        RuntimeValue::Int(10)
    );
}

#[test]
fn runtime_evaluates_domain_operator_gate_predicates() {
    let backend = lower_text(
        "backend-domain-operators-runtime.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
    type Duration -> Duration -> Bool
    (>)

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
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

    let subject = RuntimeValue::Record(vec![RuntimeRecordField {
        label: "delay".into(),
        value: RuntimeValue::SuffixedInteger {
            raw: "10".into(),
            suffix: "ms".into(),
        },
    }]);
    let faster = RuntimeValue::Record(vec![RuntimeRecordField {
        label: "delay".into(),
        value: RuntimeValue::SuffixedInteger {
            raw: "6".into(),
            suffix: "ms".into(),
        },
    }]);
    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_kernel(*predicate, Some(&subject), &[], &BTreeMap::new())
            .expect("matching duration gate predicate should evaluate"),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        evaluator
            .evaluate_kernel(*predicate, Some(&faster), &[], &BTreeMap::new())
            .expect("non-matching duration gate predicate should evaluate"),
        RuntimeValue::Bool(false)
    );
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
fn lowering_rejects_unsupported_local_references_in_source_kernels() {
    let core = manual_core_source_argument_signal(
        CoreType::Primitive(BuiltinType::Int),
        CoreExprKind::Reference(CoreReference::Local(HirBindingId::from_raw(7))),
    );
    validate_core_module(&core).expect("manual source module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let errors = lower_backend_module(&lambda)
        .expect_err("source runtime locals should fail backend lowering");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::UnsupportedLocalReference { binding, .. } if *binding == 7
    )));
}

#[test]
fn lowering_maps_open_type_parameters_to_erased_domain_layouts() {
    let open = CoreType::TypeParameter {
        parameter: HirTypeParameterId::from_raw(0),
        name: "a".into(),
    };
    let core = manual_core_gate_stage(
        open.clone(),
        open.clone(),
        {
            let open = open.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: open.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
        {
            let open = open.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: open.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual open-type module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda)
        .expect("open type parameters should lower via erased Domain layouts");
    // The open type parameter `a` maps to a Domain layout so the kernel can be compiled;
    // call sites are monomorphic by the time they reach the runtime.
    let has_domain_a = backend.layouts().iter().any(|(_, layout)| {
        matches!(&layout.kind, LayoutKind::Domain { name, .. } if name.as_ref() == "a")
    });
    assert!(
        has_domain_a,
        "open type parameter should be lowered to an erased Domain layout"
    );
}

#[test]
fn lowering_rejects_global_item_cycles() {
    let span = SourceSpan::default();
    let mut core = CoreModule::new();
    let left = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(0),
            span,
            name: "left".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("left item allocation should fit");
    let right = core
        .items_mut()
        .alloc(CoreItem {
            origin: HirItemId::from_raw(1),
            span,
            name: "right".into(),
            kind: CoreItemKind::Value,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        })
        .expect("right item allocation should fit");
    let left_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Reference(CoreReference::Item(right)),
        })
        .expect("left body allocation should fit");
    let right_body = core
        .exprs_mut()
        .alloc(CoreExpr {
            span,
            ty: CoreType::Primitive(BuiltinType::Int),
            kind: CoreExprKind::Reference(CoreReference::Item(left)),
        })
        .expect("right body allocation should fit");
    core.items_mut()
        .get_mut(left)
        .expect("left item should exist")
        .body = Some(left_body);
    core.items_mut()
        .get_mut(right)
        .expect("right item should exist")
        .body = Some(right_body);

    validate_core_module(&core).expect("manual cyclic module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let errors =
        lower_backend_module(&lambda).expect_err("global item cycles should fail lowering");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        LoweringError::GlobalItemCycle { item } if *item == aivi_backend::ItemId::from_raw(0)
            || *item == aivi_backend::ItemId::from_raw(1)
    )));
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
    let ptr = clif_pointer_ty();

    let user_type = CoreType::Record(vec![
        CoreRecordField {
            name: "active".into(),
            ty: CoreType::Primitive(BuiltinType::Bool),
        },
        CoreRecordField {
            name: "email".into(),
            ty: CoreType::Primitive(BuiltinType::Text),
        },
    ]);
    let option_user = CoreType::Option(Box::new(user_type.clone()));
    let ordinary_core = manual_core_gate_stage(
        user_type.clone(),
        option_user.clone(),
        {
            let option_user = option_user.clone();
            let user_type = user_type.clone();
            move |module, span| {
                let subject = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: user_type.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit");
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_user.clone(),
                        kind: CoreExprKind::OptionSome { payload: subject },
                    })
                    .expect("some allocation should fit")
            }
        },
        {
            let option_user = option_user.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_user.clone(),
                        kind: CoreExprKind::OptionNone,
                    })
                    .expect("none allocation should fit")
            }
        },
    );
    validate_core_module(&ordinary_core).expect("manual core module should validate");
    let ordinary_lambda =
        lower_lambda_module(&ordinary_core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&ordinary_lambda).expect("typed lambda should validate");
    let ordinary_backend =
        lower_backend_module(&ordinary_lambda).expect("backend lowering should succeed");
    validate_program(&ordinary_backend).expect("backend program should validate");

    let ordinary_item = find_item(&ordinary_backend, "captured");
    let (when_true, when_false) = match &ordinary_backend.pipelines()
        [first_pipeline(&ordinary_backend, ordinary_item)]
    .stages[0]
        .kind
    {
        BackendStageKind::Gate(BackendGateStage::Ordinary {
            when_true,
            when_false,
        }) => (*when_true, *when_false),
        other => panic!("expected ordinary gate stage, found {other:?}"),
    };
    let ordinary_compiled = compile_program(&ordinary_backend)
        .expect("record projection and Option carriers should compile");

    let gate_true = ordinary_compiled
        .kernel(when_true)
        .expect("gate-true artifact should exist");
    assert!(gate_true.code_size > 0);
    assert!(gate_true.clif.contains(&format!("({ptr}) -> {ptr}")));

    let gate_false = ordinary_compiled
        .kernel(when_false)
        .expect("gate-false artifact should exist");
    assert!(gate_false.code_size > 0);
    assert!(gate_false.clif.contains(&format!("iconst.{ptr} 0")));

    let signal_filter_core = manual_core_signal_filter_stage(
        user_type.clone(),
        user_type.clone(),
        user_type.clone(),
        move |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Projection {
                        base: CoreProjectionBase::AmbientSubject,
                        path: vec!["active".into()],
                    },
                })
                .expect("projection allocation should fit")
        },
    );
    validate_core_module(&signal_filter_core).expect("manual core module should validate");
    let signal_filter_lambda =
        lower_lambda_module(&signal_filter_core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&signal_filter_lambda).expect("typed lambda should validate");
    let signal_filter_backend =
        lower_backend_module(&signal_filter_lambda).expect("backend lowering should succeed");
    validate_program(&signal_filter_backend).expect("backend program should validate");

    let filtered = find_item(&signal_filter_backend, "filtered");
    let predicate = match &signal_filter_backend.pipelines()
        [first_pipeline(&signal_filter_backend, filtered)]
    .stages[0]
        .kind
    {
        BackendStageKind::Gate(BackendGateStage::SignalFilter { predicate, .. }) => *predicate,
        other => panic!("expected signal-filter gate stage, found {other:?}"),
    };
    let signal_filter_compiled =
        compile_program(&signal_filter_backend).expect("signal-filter predicate should compile");

    let predicate_artifact = signal_filter_compiled
        .kernel(predicate)
        .expect("predicate artifact should exist");
    assert!(predicate_artifact.code_size > 0);
    assert!(predicate_artifact.clif.contains(&format!("({ptr}) -> i8")));
    assert!(predicate_artifact.clif.contains("load.i8"));
    assert!(!ordinary_compiled.object().is_empty());
    assert!(!signal_filter_compiled.object().is_empty());
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
fn cranelift_codegen_compiles_static_text_item_bodies() {
    let backend = lower_text(
        "backend-static-text-codegen.aivi",
        r#"
value greeting:Text = "hello"
"#,
    );

    let greeting = find_item(&backend, "greeting");
    let body = backend.items()[greeting]
        .body
        .expect("greeting should carry a body kernel");

    let compiled = compile_program(&backend).expect("static text item bodies should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain greeting kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(artifact.clif.contains("symbol_value"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_static_interpolated_text_item_bodies() {
    let backend = lower_text(
        "backend-static-interpolated-text-codegen.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration

type Status =
  | Idle
  | Ready Int

value folded:Text =
    "count={7} ok={True} ratio={3.5} cost={19.25d} big={123n} dur={15ms} pair={(7, False)} list={[7, 8]} maybe={Some 7} status={Ready 9} not={not False} cmp={3 < 5} fcmp={3.5 >= 2.0} same={(Some 7) == (Some 7)} diff={(Ready 9) != (Ready 8)}"
"#,
    );

    let folded = find_item(&backend, "folded");
    let body = backend.items()[folded]
        .body
        .expect("folded should carry a body kernel");

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(folded, &BTreeMap::new())
            .expect("static interpolation should evaluate"),
        RuntimeValue::Text(
            "count=7 ok=True ratio=3.5 cost=19.25d big=123n dur=15ms pair=(7, False) list=[7, 8] maybe=Some 7 status=Ready 9 not=True cmp=True fcmp=True same=True diff=True".into()
        )
    );

    let compiled =
        compile_program(&backend).expect("static interpolated text item bodies should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain folded kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(artifact.clif.contains("symbol_value"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_static_interpolated_text_with_bytes_intrinsics() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use aivi.core.bytes (
    append,
    empty,
    get,
    length,
    repeat,
    slice,
    toText
)

value folded:Text =
    "empty={empty} len={length (append (repeat 65 1) (repeat 66 2))} get={get 1 (repeat 67 3)} slice={slice 1 3 (repeat 68 4)} text={toText (repeat 69 2)} repeat={repeat 65 3} raw={append (repeat 65 1) (repeat 66 2)}"
"#,
    );

    let folded = find_item(&backend, "folded");
    let body = backend.items()[folded]
        .body
        .expect("folded should carry a body kernel");

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(folded, &BTreeMap::new())
            .expect("bytes interpolation should evaluate"),
        RuntimeValue::Text(
            "empty=<bytes:0> len=3 get=Some 67 slice=<bytes:2> text=Some EE repeat=<bytes:3> raw=<bytes:3>".into()
        )
    );

    let compiled = compile_program(&backend)
        .expect("static bytes interpolation should fold into a native text literal");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain folded bytes kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(artifact.clif.contains("symbol_value"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_interpolated_text() {
    let backend = lower_text(
        "backend-interpolated-text-codegen.aivi",
        r#"
value host:Text = "api.example.com"
value url:Text = "https://{host}/users"
"#,
    );

    compile_program(&backend)
        .expect("interpolated text should now compile via runtime text concat");
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
fn cranelift_codegen_compiles_inline_pipe_memos() {
    let before_binding = HirBindingId::from_raw(101);
    let after_binding = HirBindingId::from_raw(102);
    let core = manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Bool),
        move |module, span| {
            let head = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::AmbientSubject,
                })
                .expect("pipe head allocation should fit");
            let stage_subject = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::AmbientSubject,
                })
                .expect("stage subject allocation should fit");
            let one = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
                })
                .expect("increment allocation should fit");
            let incremented = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Binary {
                        left: stage_subject,
                        operator: HirBinaryOperator::Add,
                        right: one,
                    },
                })
                .expect("increment expression allocation should fit");
            let before = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Reference(CoreReference::Local(before_binding)),
                })
                .expect("memo reference allocation should fit");
            let after = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Int),
                    kind: CoreExprKind::Reference(CoreReference::Local(after_binding)),
                })
                .expect("memo reference allocation should fit");
            let compared = module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Binary {
                        left: before,
                        operator: HirBinaryOperator::LessThan,
                        right: after,
                    },
                })
                .expect("comparison allocation should fit");
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                        head,
                        stages: vec![
                            CoreInlinePipeStage {
                                span,
                                subject_memo: Some(before_binding),
                                result_memo: Some(after_binding),
                                input_subject: CoreType::Primitive(BuiltinType::Int),
                                result_subject: CoreType::Primitive(BuiltinType::Int),
                                kind: CoreInlinePipeStageKind::Transform {
                                    mode: PipeTransformMode::Apply,
                                    expr: incremented,
                                },
                            },
                            CoreInlinePipeStage {
                                span,
                                subject_memo: None,
                                result_memo: None,
                                input_subject: CoreType::Primitive(BuiltinType::Int),
                                result_subject: CoreType::Primitive(BuiltinType::Bool),
                                kind: CoreInlinePipeStageKind::Transform {
                                    mode: PipeTransformMode::Replace,
                                    expr: compared,
                                },
                            },
                        ],
                    }),
                })
                .expect("pipe allocation should fit")
        },
        |module, span| {
            module
                .exprs_mut()
                .alloc(CoreExpr {
                    span,
                    ty: CoreType::Primitive(BuiltinType::Bool),
                    kind: CoreExprKind::Reference(CoreReference::Builtin(HirBuiltinTerm::False)),
                })
                .expect("fallback allocation should fit")
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

    let kernel = &backend.kernels()[*when_true];
    let KernelExprKind::Pipe(pipe) = &kernel.exprs()[kernel.root].kind else {
        panic!("expected gate kernel root to stay an inline pipe");
    };
    assert_eq!(pipe.stages.len(), 2);
    assert!(pipe.stages[0].subject_memo.is_some());
    assert!(pipe.stages[0].result_memo.is_some());

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_kernel(
                *when_true,
                Some(&RuntimeValue::Int(3)),
                &[],
                &BTreeMap::new()
            )
            .expect("inline pipe memos should evaluate"),
        RuntimeValue::Bool(true)
    );

    let compiled = compile_program(&backend).expect("inline pipe memo kernels should compile");
    let artifact = compiled
        .kernel(*when_true)
        .expect("compiled program should retain memo kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains("iadd"));
    assert!(artifact.clif.contains("icmp slt"));
    assert!(artifact.clif.contains("(i64) -> i8"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_inline_pipe_gate_option_carriers() {
    let backend = lower_text(
        "backend-inline-pipe-gate-carriers.aivi",
        r#"
value maybePositive : Option Int = 2
 ?|> True

value missingNumber : Option Int = 2
 ?|> False

value maybeGreeting : Option Text = "hello"
 ?|> True

value missingGreeting : Option Text = "hello"
 ?|> False
"#,
    );

    let maybe_positive = find_item(&backend, "maybePositive");
    let maybe_positive_body = backend.items()[maybe_positive]
        .body
        .expect("maybePositive should carry a body kernel");
    let kernel = &backend.kernels()[maybe_positive_body];
    let KernelExprKind::Pipe(pipe) = &kernel.exprs()[kernel.root].kind else {
        panic!("expected inline pipe body for maybePositive");
    };
    assert!(matches!(
        pipe.stages[0].kind,
        InlinePipeStageKind::Gate { .. }
    ));

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(maybe_positive, &globals)
            .expect("inline scalar gate should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "missingNumber"), &globals)
            .expect("inline scalar false gate should evaluate"),
        RuntimeValue::OptionNone
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "maybeGreeting"), &globals)
            .expect("inline niche gate should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Text("hello".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "missingGreeting"), &globals)
            .expect("inline niche false gate should evaluate"),
        RuntimeValue::OptionNone
    );

    let compiled = compile_program(&backend)
        .expect("inline pipe gate carriers should compile through Cranelift");

    let maybe_positive_artifact = compiled
        .kernel(maybe_positive_body)
        .expect("compiled program should retain maybePositive metadata");
    assert!(maybe_positive_artifact.code_size > 0);
    assert!(maybe_positive_artifact.clif.contains("brif"));
    assert!(maybe_positive_artifact.clif.contains("() -> i128"));
    assert!(maybe_positive_artifact.clif.contains("ishl_imm"));

    let missing_number_body = backend.items()[find_item(&backend, "missingNumber")]
        .body
        .expect("missingNumber should carry a body kernel");
    let missing_number_artifact = compiled
        .kernel(missing_number_body)
        .expect("compiled program should retain missingNumber metadata");
    assert!(missing_number_artifact.clif.contains("iconst.i64 0"));
    assert!(missing_number_artifact.clif.contains("uextend.i128"));

    let maybe_greeting_body = backend.items()[find_item(&backend, "maybeGreeting")]
        .body
        .expect("maybeGreeting should carry a body kernel");
    let maybe_greeting_artifact = compiled
        .kernel(maybe_greeting_body)
        .expect("compiled program should retain maybeGreeting metadata");
    let ptr = clif_pointer_ty();
    assert!(maybe_greeting_artifact.code_size > 0);
    assert!(maybe_greeting_artifact.clif.contains("brif"));
    assert!(
        maybe_greeting_artifact
            .clif
            .contains(&format!("() -> {ptr}"))
    );
    assert!(maybe_greeting_artifact.clif.contains("symbol_value"));

    let missing_greeting_body = backend.items()[find_item(&backend, "missingGreeting")]
        .body
        .expect("missingGreeting should carry a body kernel");
    let missing_greeting_artifact = compiled
        .kernel(missing_greeting_body)
        .expect("compiled program should retain missingGreeting metadata");
    assert!(
        missing_greeting_artifact
            .clif
            .contains(&format!("iconst.{ptr} 0"))
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_float_comparison_and_equality_kernels() {
    let backend = lower_text(
        "backend-float-compare-codegen.aivi",
        r#"
fun gt:Bool = left:Float right:Float=>    left > right

fun eq:Bool = left:Float right:Float=>    left == right

fun ne:Bool = left:Float right:Float=>    left != right
"#,
    );

    let gt_body = backend.items()[find_item(&backend, "gt")]
        .body
        .expect("gt should carry a body kernel");
    let eq_body = backend.items()[find_item(&backend, "eq")]
        .body
        .expect("eq should carry a body kernel");
    let ne_body = backend.items()[find_item(&backend, "ne")]
        .body
        .expect("ne should carry a body kernel");

    let compiled =
        compile_program(&backend).expect("Float compare/equality kernels should compile");

    let gt_artifact = compiled
        .kernel(gt_body)
        .expect("compiled program should retain gt kernel metadata");
    assert!(gt_artifact.code_size > 0);
    assert!(gt_artifact.clif.contains("(f64, f64) -> i8"));
    assert!(gt_artifact.clif.contains("fcmp gt"));

    let eq_artifact = compiled
        .kernel(eq_body)
        .expect("compiled program should retain eq kernel metadata");
    assert!(eq_artifact.code_size > 0);
    assert!(eq_artifact.clif.contains("(f64, f64) -> i8"));
    assert!(eq_artifact.clif.contains("fcmp eq"));

    let ne_artifact = compiled
        .kernel(ne_body)
        .expect("compiled program should retain ne kernel metadata");
    assert!(ne_artifact.code_size > 0);
    assert!(ne_artifact.clif.contains("(f64, f64) -> i8"));
    assert!(ne_artifact.clif.contains("fcmp ne"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_native_tuple_equality_kernels() {
    let backend = lower_text(
        "backend-tuple-equality-codegen.aivi",
        r#"
fun sameTuple:Bool = left:(Int, Float, Bool) right:(Int, Float, Bool)=>    left == right

fun differentTuple:Bool = left:(Int, Float, Bool) right:(Int, Float, Bool)=>    left != right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_body = backend.items()[find_item(&backend, "sameTuple")]
        .body
        .expect("sameTuple should carry a body kernel");
    let different_body = backend.items()[find_item(&backend, "differentTuple")]
        .body
        .expect("differentTuple should carry a body kernel");

    let compiled =
        compile_program(&backend).expect("native tuple equality over scalar leaves should compile");

    let same_artifact = compiled
        .kernel(same_body)
        .expect("compiled program should retain sameTuple kernel metadata");
    assert!(same_artifact.code_size > 0);
    assert!(
        same_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_artifact.clif.contains("load.i64"));
    assert!(same_artifact.clif.contains("load.f64"));
    assert!(same_artifact.clif.contains("load.i8"));
    assert!(same_artifact.clif.contains("band"));

    let different_artifact = compiled
        .kernel(different_body)
        .expect("compiled program should retain differentTuple kernel metadata");
    assert!(different_artifact.code_size > 0);
    assert!(
        different_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(different_artifact.clif.contains("bxor"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_native_record_equality_kernels() {
    let backend = lower_text(
        "backend-record-equality-codegen.aivi",
        r#"
type Stats = { count: Int, ratio: Float, active: Bool }
type Wrapper = { stats: Stats, enabled: Bool }

fun sameStats:Bool = left:Stats right:Stats=>    left == right

fun differentWrapper:Bool = left:Wrapper right:Wrapper=>    left != right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_stats_body = backend.items()[find_item(&backend, "sameStats")]
        .body
        .expect("sameStats should carry a body kernel");
    let different_wrapper_body = backend.items()[find_item(&backend, "differentWrapper")]
        .body
        .expect("differentWrapper should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("native record equality over scalar leaves should compile");

    let same_artifact = compiled
        .kernel(same_stats_body)
        .expect("compiled program should retain sameStats kernel metadata");
    assert!(same_artifact.code_size > 0);
    assert!(
        same_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_artifact.clif.contains("load.i64"));
    assert!(same_artifact.clif.contains("load.f64"));
    assert!(same_artifact.clif.contains("load.i8"));
    assert!(same_artifact.clif.contains("band"));

    let different_artifact = compiled
        .kernel(different_wrapper_body)
        .expect("compiled program should retain differentWrapper kernel metadata");
    assert!(different_artifact.code_size > 0);
    assert!(
        different_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(different_artifact.clif.contains("load.i64"));
    assert!(different_artifact.clif.contains("load.f64"));
    assert!(different_artifact.clif.contains("load.i8"));
    assert!(different_artifact.clif.contains("bxor"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_niche_option_equality_kernels() {
    let backend = lower_text(
        "backend-option-equality-codegen.aivi",
        r#"
type Stats = { count: Int, active: Bool }

fun sameMaybeStats:Bool = left:(Option Stats) right:(Option Stats)=>    left == right

fun differentMaybeStats:Bool = left:(Option Stats) right:(Option Stats)=>    left != right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_body = backend.items()[find_item(&backend, "sameMaybeStats")]
        .body
        .expect("sameMaybeStats should carry a body kernel");
    let different_body = backend.items()[find_item(&backend, "differentMaybeStats")]
        .body
        .expect("differentMaybeStats should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("niche Option equality over supported payloads should compile");

    let same_artifact = compiled
        .kernel(same_body)
        .expect("compiled program should retain sameMaybeStats kernel metadata");
    assert!(same_artifact.code_size > 0);
    assert!(
        same_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_artifact.clif.contains("brif"));
    assert!(same_artifact.clif.contains("load.i64"));
    assert!(same_artifact.clif.contains("load.i8"));

    let different_artifact = compiled
        .kernel(different_body)
        .expect("compiled program should retain differentMaybeStats kernel metadata");
    assert!(different_artifact.code_size > 0);
    assert!(
        different_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(different_artifact.clif.contains("brif"));
    assert!(different_artifact.clif.contains("bxor"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_nested_native_equality_kernels() {
    let backend = lower_text(
        "backend-nested-equality-codegen.aivi",
        r#"
type Pair = (Int, Float, Bool)
type Stats = {
    pair: Pair,
    maybePair: Option Pair
}

fun sameStats:Bool = left:Stats right:Stats=>    left == right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_body = backend.items()[find_item(&backend, "sameStats")]
        .body
        .expect("sameStats should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("nested native equality over supported shapes should compile");

    let artifact = compiled
        .kernel(same_body)
        .expect("compiled program should retain sameStats kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains(&format!("({ptr}, {ptr}) -> i8")));
    assert!(artifact.clif.contains("brif"));
    assert!(artifact.clif.contains("load.i64"));
    assert!(artifact.clif.contains("load.f64"));
    assert!(artifact.clif.contains("load.i8"));
    assert!(artifact.clif.contains("band"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_scalar_option_equality_and_constructors() {
    let backend = lower_text(
        "backend-option-int-equality-codegen.aivi",
        r#"
fun sameMaybeInt:Bool = left:(Option Int) right:(Option Int)=>    left == right

fun differentMaybeInt:Bool = left:(Option Int) right:(Option Int)=>    left != right

fun liftMaybeInt:(Option Int) = value:Int=>    Some value

value missingMaybeInt:(Option Int) =
    None
"#,
    );

    let same_body = backend.items()[find_item(&backend, "sameMaybeInt")]
        .body
        .expect("sameMaybeInt should carry a body kernel");
    let different_body = backend.items()[find_item(&backend, "differentMaybeInt")]
        .body
        .expect("differentMaybeInt should carry a body kernel");
    let lift_body = backend.items()[find_item(&backend, "liftMaybeInt")]
        .body
        .expect("liftMaybeInt should carry a body kernel");
    let missing_body = backend.items()[find_item(&backend, "missingMaybeInt")]
        .body
        .expect("missingMaybeInt should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("scalar Option Int constructors and equality should compile");

    let same_artifact = compiled
        .kernel(same_body)
        .expect("compiled program should retain sameMaybeInt kernel metadata");
    assert!(same_artifact.code_size > 0);
    assert!(same_artifact.clif.contains("(i128, i128) -> i8"));
    assert!(same_artifact.clif.contains("icmp"));

    let different_artifact = compiled
        .kernel(different_body)
        .expect("compiled program should retain differentMaybeInt kernel metadata");
    assert!(different_artifact.code_size > 0);
    assert!(different_artifact.clif.contains("(i128, i128) -> i8"));
    assert!(different_artifact.clif.contains("bxor"));

    let lift_artifact = compiled
        .kernel(lift_body)
        .expect("compiled program should retain liftMaybeInt kernel metadata");
    assert!(lift_artifact.code_size > 0);
    assert!(lift_artifact.clif.contains("(i64) -> i128"));
    assert!(lift_artifact.clif.contains("ishl_imm"));
    assert!(lift_artifact.clif.contains("bor"));

    let missing_artifact = compiled
        .kernel(missing_body)
        .expect("compiled program should retain missingMaybeInt kernel metadata");
    assert!(missing_artifact.code_size > 0);
    assert!(missing_artifact.clif.contains("() -> i128"));
    assert!(missing_artifact.clif.contains("iconst.i64 0"));
    assert!(missing_artifact.clif.contains("uextend.i128"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_text_equality_kernels() {
    let backend = lower_text(
        "backend-text-equality-codegen.aivi",
        r#"
fun sameText:Bool = left:Text right:Text=>    left == right

fun differentText:Bool = left:Text right:Text=>    left != right

type Labels = {
    primary: Text,
    alias: Option Text
}

fun sameLabels:Bool = left:Labels right:Labels=>    left == right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_text_body = backend.items()[find_item(&backend, "sameText")]
        .body
        .expect("sameText should carry a body kernel");
    let different_text_body = backend.items()[find_item(&backend, "differentText")]
        .body
        .expect("differentText should carry a body kernel");
    let same_labels_body = backend.items()[find_item(&backend, "sameLabels")]
        .body
        .expect("sameLabels should carry a body kernel");

    let compiled =
        compile_program(&backend).expect("Text equality over native text cells should compile");

    let same_text_artifact = compiled
        .kernel(same_text_body)
        .expect("compiled program should retain sameText kernel metadata");
    assert!(same_text_artifact.code_size > 0);
    assert!(
        same_text_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_text_artifact.clif.contains("load.i64"));
    assert!(same_text_artifact.clif.contains("load.i8"));
    assert!(same_text_artifact.clif.contains("brif"));

    let different_text_artifact = compiled
        .kernel(different_text_body)
        .expect("compiled program should retain differentText kernel metadata");
    assert!(different_text_artifact.code_size > 0);
    assert!(
        different_text_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(different_text_artifact.clif.contains("bxor"));

    let same_labels_artifact = compiled
        .kernel(same_labels_body)
        .expect("compiled program should retain sameLabels kernel metadata");
    assert!(same_labels_artifact.code_size > 0);
    assert!(
        same_labels_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_labels_artifact.clif.contains("load.i64"));
    assert!(same_labels_artifact.clif.contains("load.i8"));
    assert!(same_labels_artifact.clif.contains("band"));
    assert!(same_labels_artifact.clif.contains("brif"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_bytes_equality_kernels() {
    let backend = lower_text(
        "backend-bytes-equality-codegen.aivi",
        r#"
fun sameBytes:Bool = left:Bytes right:Bytes=>    left == right

fun differentBytes:Bool = left:Bytes right:Bytes=>    left != right

type Blobs = {
    primary: Bytes,
    alias: Option Bytes
}

fun sameBlobs:Bool = left:Blobs right:Blobs=>    left == right
"#,
    );

    let ptr = clif_pointer_ty();
    let same_bytes_body = backend.items()[find_item(&backend, "sameBytes")]
        .body
        .expect("sameBytes should carry a body kernel");
    let different_bytes_body = backend.items()[find_item(&backend, "differentBytes")]
        .body
        .expect("differentBytes should carry a body kernel");
    let same_blobs_body = backend.items()[find_item(&backend, "sameBlobs")]
        .body
        .expect("sameBlobs should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("Bytes equality over native byte-sequence cells should compile");

    let same_bytes_artifact = compiled
        .kernel(same_bytes_body)
        .expect("compiled program should retain sameBytes kernel metadata");
    assert!(same_bytes_artifact.code_size > 0);
    assert!(
        same_bytes_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_bytes_artifact.clif.contains("load.i64"));
    assert!(same_bytes_artifact.clif.contains("load.i8"));
    assert!(same_bytes_artifact.clif.contains("brif"));

    let different_bytes_artifact = compiled
        .kernel(different_bytes_body)
        .expect("compiled program should retain differentBytes kernel metadata");
    assert!(different_bytes_artifact.code_size > 0);
    assert!(
        different_bytes_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(different_bytes_artifact.clif.contains("bxor"));

    let same_blobs_artifact = compiled
        .kernel(same_blobs_body)
        .expect("compiled program should retain sameBlobs kernel metadata");
    assert!(same_blobs_artifact.code_size > 0);
    assert!(
        same_blobs_artifact
            .clif
            .contains(&format!("({ptr}, {ptr}) -> i8"))
    );
    assert!(same_blobs_artifact.clif.contains("load.i64"));
    assert!(same_blobs_artifact.clif.contains("load.i8"));
    assert!(same_blobs_artifact.clif.contains("band"));
    assert!(same_blobs_artifact.clif.contains("brif"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_selected_bytes_intrinsics() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use aivi.core.bytes (
    empty,
    get,
    fromText,
    length,
    toText
)

value noBytes:Bytes =
    empty

fun measureBytes:Int = bytes:Bytes=>    length bytes

fun firstByte:(Option Int) = bytes:Bytes=>    get 0 bytes

fun encodeLabel:Bytes = label:Text=>    fromText label

fun decodeLabel:(Option Text) = bytes:Bytes=>    toText bytes
"#,
    );

    let ptr = clif_pointer_ty();
    let no_bytes_body = backend.items()[find_item(&backend, "noBytes")]
        .body
        .expect("noBytes should carry a body kernel");
    let measure_bytes_body = backend.items()[find_item(&backend, "measureBytes")]
        .body
        .expect("measureBytes should carry a body kernel");
    let first_byte_body = backend.items()[find_item(&backend, "firstByte")]
        .body
        .expect("firstByte should carry a body kernel");
    let encode_label_body = backend.items()[find_item(&backend, "encodeLabel")]
        .body
        .expect("encodeLabel should carry a body kernel");
    let decode_label_body = backend.items()[find_item(&backend, "decodeLabel")]
        .body
        .expect("decodeLabel should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("selected bytes intrinsics should compile through Cranelift");

    let no_bytes_artifact = compiled
        .kernel(no_bytes_body)
        .expect("compiled program should retain noBytes kernel metadata");
    assert!(no_bytes_artifact.code_size > 0);
    assert!(no_bytes_artifact.clif.contains(&format!("() -> {ptr}")));
    assert!(no_bytes_artifact.clif.contains("symbol_value"));

    let measure_bytes_artifact = compiled
        .kernel(measure_bytes_body)
        .expect("compiled program should retain measureBytes kernel metadata");
    assert!(measure_bytes_artifact.code_size > 0);
    assert!(
        measure_bytes_artifact
            .clif
            .contains(&format!("({ptr}) -> i64"))
    );
    assert!(measure_bytes_artifact.clif.contains("load.i64"));

    let first_byte_artifact = compiled
        .kernel(first_byte_body)
        .expect("compiled program should retain firstByte kernel metadata");
    assert!(first_byte_artifact.code_size > 0);
    assert!(
        first_byte_artifact
            .clif
            .contains(&format!("({ptr}) -> i128"))
    );
    assert!(first_byte_artifact.clif.contains("load.i64"));
    assert!(first_byte_artifact.clif.contains("load.i8"));
    assert!(first_byte_artifact.clif.contains("brif"));

    let encode_label_artifact = compiled
        .kernel(encode_label_body)
        .expect("compiled program should retain encodeLabel kernel metadata");
    assert!(encode_label_artifact.code_size > 0);
    assert!(
        encode_label_artifact
            .clif
            .contains(&format!("({ptr}) -> {ptr}"))
    );

    let decode_label_artifact = compiled
        .kernel(decode_label_body)
        .expect("compiled program should retain decodeLabel kernel metadata");
    assert!(decode_label_artifact.code_size > 0);
    assert!(
        decode_label_artifact
            .clif
            .contains(&format!("({ptr}) -> {ptr}"))
    );
    assert!(decode_label_artifact.clif.contains("load.i64"));
    assert!(decode_label_artifact.clif.contains("load.i8"));
    assert!(decode_label_artifact.clif.contains("brif"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_materializes_supported_static_bytes_roots() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use aivi.core.bytes (
    append,
    length,
    repeat,
    toText
)

value blob:Bytes =
    append (repeat 65 1) (repeat 66 2)

value size:Int =
    length (append (repeat 65 1) (repeat 66 2))

value decoded:(Option Text) =
    toText (repeat 69 2)

value invalid:(Option Text) =
    toText (repeat 255 1)
"#,
    );

    let blob = find_item(&backend, "blob");
    let size = find_item(&backend, "size");
    let decoded = find_item(&backend, "decoded");
    let invalid = find_item(&backend, "invalid");

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(blob, &BTreeMap::new())
            .expect("blob should evaluate"),
        RuntimeValue::Bytes(Box::from(*b"ABB"))
    );
    assert_eq!(
        evaluator
            .evaluate_item(size, &BTreeMap::new())
            .expect("size should evaluate"),
        RuntimeValue::Int(3)
    );
    assert_eq!(
        evaluator
            .evaluate_item(decoded, &BTreeMap::new())
            .expect("decoded should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Text("EE".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(invalid, &BTreeMap::new())
            .expect("invalid should evaluate"),
        RuntimeValue::OptionNone
    );

    let blob_body = backend.items()[blob]
        .body
        .expect("blob should carry a body kernel");
    let size_body = backend.items()[size]
        .body
        .expect("size should carry a body kernel");
    let decoded_body = backend.items()[decoded]
        .body
        .expect("decoded should carry a body kernel");
    let invalid_body = backend.items()[invalid]
        .body
        .expect("invalid should carry a body kernel");

    let compiled = compile_program(&backend)
        .expect("supported static bytes roots should materialize through Cranelift");

    let blob_artifact = compiled
        .kernel(blob_body)
        .expect("compiled program should retain blob kernel metadata");
    assert!(blob_artifact.code_size > 0);
    assert!(
        blob_artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(blob_artifact.clif.contains("symbol_value"));

    let size_artifact = compiled
        .kernel(size_body)
        .expect("compiled program should retain size kernel metadata");
    assert!(size_artifact.code_size > 0);
    assert!(size_artifact.clif.contains("() -> i64"));

    let decoded_artifact = compiled
        .kernel(decoded_body)
        .expect("compiled program should retain decoded kernel metadata");
    assert!(decoded_artifact.code_size > 0);
    assert!(
        decoded_artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(decoded_artifact.clif.contains("symbol_value"));

    let invalid_artifact = compiled
        .kernel(invalid_body)
        .expect("compiled program should retain invalid kernel metadata");
    assert!(invalid_artifact.code_size > 0);
    assert!(
        invalid_artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_rejects_unimplemented_bytes_intrinsics() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use aivi.core.bytes (
    append
)

fun combine:Bytes = left:Bytes right:Bytes=>    append left right
"#,
    );

    let errors = compile_program(&backend)
        .expect_err("bytes.append should stay unsupported without allocation");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { detail, .. }
            if detail.contains("empty/length/get/fromText/toText Cranelift subset")
    )));
}

#[test]
fn cranelift_codegen_rejects_named_domain_equality_without_native_domain_contracts() {
    let backend = lower_text(
        "backend-domain-equality-codegen.aivi",
        r#"
domain Path over Text

fun samePath:Bool = left:Path right:Path=>    left == right
"#,
    );

    let errors =
        compile_program(&backend).expect_err("named-domain equality should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { detail, .. }
            if detail.contains("representation bridge") && detail.contains("Path")
    )));
}

#[test]
fn cranelift_codegen_compiles_inline_subject_projection_pipes() {
    let user_type = CoreType::Record(vec![
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
        user_type.clone(),
        CoreType::Primitive(BuiltinType::Bool),
        {
            let user_type = user_type.clone();
            move |module, span| {
                let head = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: user_type.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("pipe head allocation should fit");
                let projected = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: CoreType::Primitive(BuiltinType::Bool),
                        kind: CoreExprKind::Projection {
                            base: CoreProjectionBase::AmbientSubject,
                            path: vec!["active".into()],
                        },
                    })
                    .expect("projection allocation should fit");
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: CoreType::Primitive(BuiltinType::Bool),
                        kind: CoreExprKind::Pipe(CoreInlinePipeExpr {
                            head,
                            stages: vec![CoreInlinePipeStage {
                                span,
                                subject_memo: None,
                                result_memo: None,
                                input_subject: user_type.clone(),
                                result_subject: CoreType::Primitive(BuiltinType::Bool),
                                kind: CoreInlinePipeStageKind::Transform {
                                    mode: PipeTransformMode::Apply,
                                    expr: projected,
                                },
                            }],
                        }),
                    })
                    .expect("pipe allocation should fit")
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
                .expect("fallback allocation should fit")
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

    let kernel = &backend.kernels()[*when_true];
    let KernelExprKind::Pipe(pipe) = &kernel.exprs()[kernel.root].kind else {
        panic!("expected gate kernel root to stay an inline pipe");
    };
    let KernelExprKind::Projection { base, .. } = &kernel.exprs()[match &pipe.stages[0].kind {
        InlinePipeStageKind::Transform { expr, .. } => *expr,
        other => panic!("expected transform stage, found {other:?}"),
    }]
    .kind
    else {
        panic!("expected stage body to stay a projection");
    };
    assert!(matches!(
        base,
        ProjectionBase::Subject(SubjectRef::Inline(_))
    ));

    let subject = RuntimeValue::Record(vec![
        RuntimeRecordField {
            label: "active".into(),
            value: RuntimeValue::Bool(true),
        },
        RuntimeRecordField {
            label: "email".into(),
            value: RuntimeValue::Text("hello@example.com".into()),
        },
    ]);
    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_kernel(*when_true, Some(&subject), &[], &BTreeMap::new())
            .expect("inline subject projection pipes should evaluate"),
        RuntimeValue::Bool(true)
    );

    let compiled =
        compile_program(&backend).expect("inline subject projection pipes should compile");
    let artifact = compiled
        .kernel(*when_true)
        .expect("compiled program should retain projection kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains("load.i8"));
    assert!(
        artifact
            .clif
            .contains(&format!("({}) -> i8", clif_pointer_ty()))
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_item_body_kernels_and_direct_item_calls() {
    let backend = lower_text(
        "backend-item-body-codegen.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

value base = 41

value answer =
    addOne 41
"#,
    );

    let add_one = find_item(&backend, "addOne");
    let base = find_item(&backend, "base");
    let answer = find_item(&backend, "answer");
    let add_one_body = backend.items()[add_one]
        .body
        .expect("addOne should carry an item body kernel");
    let base_body = backend.items()[base]
        .body
        .expect("base should carry an item body kernel");
    let answer_body = backend.items()[answer]
        .body
        .expect("answer should carry an item body kernel");

    let compiled =
        compile_program(&backend).expect("item body kernels and direct item calls should compile");

    let add_one_artifact = compiled
        .kernel(add_one_body)
        .expect("compiled program should retain addOne item body metadata");
    assert!(add_one_artifact.code_size > 0);
    assert!(add_one_artifact.symbol.contains("item_body"));
    assert!(add_one_artifact.clif.contains("(i64) -> i64"));

    let base_artifact = compiled
        .kernel(base_body)
        .expect("compiled program should retain base item body metadata");
    assert!(base_artifact.code_size > 0);
    assert!(base_artifact.symbol.contains("item_body"));
    assert!(base_artifact.clif.contains("() -> i64"));

    let answer_artifact = compiled
        .kernel(answer_body)
        .expect("compiled program should retain answer item body metadata");
    assert!(answer_artifact.code_size > 0);
    assert!(answer_artifact.symbol.contains("item_body"));
    assert!(answer_artifact.clif.contains("call"));
    assert!(answer_artifact.clif.contains("() -> i64"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_direct_builtin_option_calls() {
    let text = CoreType::Primitive(BuiltinType::Text);
    let option_text = CoreType::Option(Box::new(text.clone()));
    let some_ty = CoreType::Arrow {
        parameter: Box::new(text.clone()),
        result: Box::new(option_text.clone()),
    };
    let core = manual_core_gate_stage(
        text.clone(),
        option_text.clone(),
        {
            let text = text.clone();
            let option_text = option_text.clone();
            let some_ty = some_ty.clone();
            move |module, span| {
                let callee = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: some_ty.clone(),
                        kind: CoreExprKind::Reference(CoreReference::Builtin(HirBuiltinTerm::Some)),
                    })
                    .expect("Some callee allocation should fit");
                let subject = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: text.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("ambient text subject allocation should fit");
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_text.clone(),
                        kind: CoreExprKind::Apply {
                            callee,
                            arguments: vec![subject],
                        },
                    })
                    .expect("Some apply allocation should fit")
            }
        },
        {
            let option_text = option_text.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_text.clone(),
                        kind: CoreExprKind::Reference(CoreReference::Builtin(HirBuiltinTerm::None)),
                    })
                    .expect("None builtin allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual builtin constructor module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let (when_true, when_false) =
        match &backend.pipelines()[first_pipeline(&backend, item)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::Ordinary {
                when_true,
                when_false,
            }) => (*when_true, *when_false),
            other => panic!("expected ordinary gate stage, found {other:?}"),
        };

    let some_kernel = &backend.kernels()[when_true];
    match &some_kernel.exprs()[some_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &some_kernel.exprs()[*callee].kind,
                KernelExprKind::Builtin(BuiltinTerm::Some)
            ));
        }
        other => panic!("expected Some body to lower into an apply tree, found {other:?}"),
    }

    let none_kernel = &backend.kernels()[when_false];
    assert!(matches!(
        &none_kernel.exprs()[none_kernel.root].kind,
        KernelExprKind::Builtin(BuiltinTerm::None)
    ));

    let compiled =
        compile_program(&backend).expect("niche Option builtin constructors should compile");
    let ptr = clif_pointer_ty();

    let some_artifact = compiled
        .kernel(when_true)
        .expect("compiled program should retain Some kernel metadata");
    assert!(some_artifact.code_size > 0);
    assert!(some_artifact.clif.contains(&format!("({ptr}) -> {ptr}")));
    assert!(!some_artifact.clif.contains("call"));

    let none_artifact = compiled
        .kernel(when_false)
        .expect("compiled program should retain None kernel metadata");
    assert!(none_artifact.code_size > 0);
    assert!(none_artifact.clif.contains(&format!("() -> {ptr}")));
    assert!(none_artifact.clif.contains(&format!("iconst.{ptr} 0")));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_direct_by_reference_domain_member_calls() {
    let backend = lower_text(
        "backend-domain-member-codegen.aivi",
        r#"
domain Path over Text
    type Text -> Path
    fromText
    type Path -> Text
    unwrap

fun wrapPath:Path = raw:Text=>    fromText raw

fun unwrapPath:Text = path:Path=>    unwrap path
"#,
    );

    let wrap_path = find_item(&backend, "wrapPath");
    let unwrap_path = find_item(&backend, "unwrapPath");
    let wrap_body = backend.items()[wrap_path]
        .body
        .expect("wrapPath should carry a body kernel");
    let unwrap_body = backend.items()[unwrap_path]
        .body
        .expect("unwrapPath should carry a body kernel");

    let wrap_kernel = &backend.kernels()[wrap_body];
    match &wrap_kernel.exprs()[wrap_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &wrap_kernel.exprs()[*callee].kind,
                KernelExprKind::DomainMember(handle)
                    if handle.domain_name.as_ref() == "Path"
                        && handle.member_name.as_ref() == "fromText"
            ));
        }
        other => {
            panic!("expected wrapPath body to lower into a domain-member apply, found {other:?}")
        }
    }

    let unwrap_kernel = &backend.kernels()[unwrap_body];
    match &unwrap_kernel.exprs()[unwrap_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &unwrap_kernel.exprs()[*callee].kind,
                KernelExprKind::DomainMember(handle)
                    if handle.domain_name.as_ref() == "Path"
                        && handle.member_name.as_ref() == "unwrap"
            ));
        }
        other => {
            panic!("expected unwrapPath body to lower into a domain-member apply, found {other:?}")
        }
    }

    let compiled = compile_program(&backend)
        .expect("representational by-reference domain member calls should compile");
    let ptr = clif_pointer_ty();

    for body in [wrap_body, unwrap_body] {
        let artifact = compiled
            .kernel(body)
            .expect("compiled program should retain domain-member kernel metadata");
        assert!(artifact.code_size > 0);
        assert!(artifact.clif.contains(&format!("({ptr}) -> {ptr}")));
        assert!(!artifact.clif.contains("call"));
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn runtime_and_codegen_accept_domain_dot_projection_over_values() {
    let backend = lower_text(
        "backend-domain-dot-projection.aivi",
        r#"
domain Path over Text
    type Text -> Path
    fromText
    type Path -> Text
    unwrap

value home : Path = fromText "/tmp/app"
value raw : Text = home.unwrap
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "raw"), &BTreeMap::new())
            .expect("domain dot-projection should evaluate"),
        RuntimeValue::Text("/tmp/app".into())
    );

    let compiled = compile_program(&backend)
        .expect("representational domain dot-projection should compile through Cranelift");
    let raw = find_item(&backend, "raw");
    let raw_body = backend.items()[raw]
        .body
        .expect("raw should carry a body kernel");
    let artifact = compiled
        .kernel(raw_body)
        .expect("compiled program should retain domain dot-projection kernel metadata");
    assert!(artifact.code_size > 0);
}

#[test]
fn cranelift_codegen_rejects_unsaturated_direct_item_apply_calls() {
    let backend = lower_text(
        "backend-item-body-partial-apply-codegen.aivi",
        r#"
fun add:Int = left:Int right:Int=>    left + right

value addOne =
    add 1
"#,
    );

    let errors = compile_program(&backend).expect_err("partial item apply should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { detail, .. }
            if detail.contains("requires saturation")
    )));
}

#[test]
fn cranelift_codegen_compiles_result_constructors() {
    let backend = lower_text(
        "backend-result-constructor-codegen.aivi",
        r#"
fun wrapOk:(Result Text Text) = text:Text=>    Ok text
"#,
    );

    compile_program(&backend).expect("Result constructors should now compile via SumConstruction");
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
fn cranelift_codegen_compiles_scalar_option_carriers() {
    let option_int = CoreType::Option(Box::new(CoreType::Primitive(BuiltinType::Int)));
    let core = manual_core_gate_stage(
        CoreType::Primitive(BuiltinType::Bool),
        option_int.clone(),
        {
            let option_int = option_int.clone();
            move |module, span| {
                let payload = module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: CoreType::Primitive(BuiltinType::Int),
                        kind: CoreExprKind::Integer(IntegerLiteral { raw: "1".into() }),
                    })
                    .expect("payload allocation should fit");
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_int.clone(),
                        kind: CoreExprKind::OptionSome { payload },
                    })
                    .expect("Some allocation should fit")
            }
        },
        {
            let option_int = option_int.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: option_int.clone(),
                        kind: CoreExprKind::OptionNone,
                    })
                    .expect("None allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual option module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let (when_true, when_false) =
        match &backend.pipelines()[first_pipeline(&backend, item)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::Ordinary {
                when_true,
                when_false,
            }) => (*when_true, *when_false),
            other => panic!("expected ordinary gate stage, found {other:?}"),
        };

    let compiled =
        compile_program(&backend).expect("scalar Option carriers should compile through Cranelift");
    let when_true_artifact = compiled
        .kernel(when_true)
        .expect("compiled program should retain Some carrier kernel metadata");
    assert!(when_true_artifact.code_size > 0);
    assert!(when_true_artifact.clif.contains("() -> i128"));
    assert!(when_true_artifact.clif.contains("ishl_imm"));
    assert!(when_true_artifact.clif.contains("bor"));

    let when_false_artifact = compiled
        .kernel(when_false)
        .expect("compiled program should retain None carrier kernel metadata");
    assert!(when_false_artifact.code_size > 0);
    assert!(when_false_artifact.clif.contains("() -> i128"));
    assert!(when_false_artifact.clif.contains("iconst.i64 0"));
    assert!(when_false_artifact.clif.contains("uextend.i128"));
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_rejects_by_value_text_abi_contracts() {
    let text = CoreType::Primitive(BuiltinType::Text);
    let core = manual_core_gate_stage(
        text.clone(),
        text.clone(),
        {
            let text = text.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: text.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
        {
            let text = text.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: text.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual text module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let mut backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let (when_true, when_false) =
        match &backend.pipelines()[first_pipeline(&backend, item)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::Ordinary {
                when_true,
                when_false,
            }) => (*when_true, *when_false),
            other => panic!("expected ordinary gate stage, found {other:?}"),
        };
    let text_layout = backend.kernels()[when_true]
        .input_subject
        .expect("text gate kernels should keep an input subject");
    backend
        .layouts_mut()
        .get_mut(text_layout)
        .expect("text layout should exist")
        .abi = AbiPassMode::ByValue;
    for kernel_id in [when_true, when_false] {
        let kernel = backend
            .kernels_mut()
            .get_mut(kernel_id)
            .expect("gate kernel should exist");
        kernel.convention.parameters[0].pass_mode = AbiPassMode::ByValue;
        kernel.convention.result.pass_mode = AbiPassMode::ByValue;
    }
    validate_program(&backend).expect("manually aligned text ABI should stay backend-valid");

    let errors = compile_program(&backend).expect_err("by-value Text ABI should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedLayout { kernel, layout, detail }
            if (*kernel == when_true || *kernel == when_false)
                && *layout == text_layout
                && detail.contains("uses primitive `Text`")
    )));
}

#[test]
fn cranelift_codegen_rejects_by_value_aggregate_abi_contracts() {
    let pair = CoreType::Tuple(vec![
        CoreType::Primitive(BuiltinType::Int),
        CoreType::Primitive(BuiltinType::Bool),
    ]);
    let core = manual_core_gate_stage(
        pair.clone(),
        pair.clone(),
        {
            let pair = pair.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: pair.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
        {
            let pair = pair.clone();
            move |module, span| {
                module
                    .exprs_mut()
                    .alloc(CoreExpr {
                        span,
                        ty: pair.clone(),
                        kind: CoreExprKind::AmbientSubject,
                    })
                    .expect("subject allocation should fit")
            }
        },
    );
    validate_core_module(&core).expect("manual aggregate module should validate");
    let lambda = lower_lambda_module(&core).expect("typed lambda lowering should succeed");
    validate_lambda_module(&lambda).expect("typed lambda should validate");
    let mut backend = lower_backend_module(&lambda).expect("backend lowering should succeed");
    validate_program(&backend).expect("backend program should validate");

    let item = find_item(&backend, "captured");
    let (when_true, when_false) =
        match &backend.pipelines()[first_pipeline(&backend, item)].stages[0].kind {
            BackendStageKind::Gate(BackendGateStage::Ordinary {
                when_true,
                when_false,
            }) => (*when_true, *when_false),
            other => panic!("expected ordinary gate stage, found {other:?}"),
        };
    let pair_layout = backend.kernels()[when_true]
        .input_subject
        .expect("aggregate gate kernels should keep an input subject");
    backend
        .layouts_mut()
        .get_mut(pair_layout)
        .expect("aggregate layout should exist")
        .abi = AbiPassMode::ByValue;
    for kernel_id in [when_true, when_false] {
        let kernel = backend
            .kernels_mut()
            .get_mut(kernel_id)
            .expect("gate kernel should exist");
        kernel.convention.parameters[0].pass_mode = AbiPassMode::ByValue;
        kernel.convention.result.pass_mode = AbiPassMode::ByValue;
    }
    validate_program(&backend).expect("manually aligned aggregate ABI should stay backend-valid");

    let errors =
        compile_program(&backend).expect_err("by-value aggregate ABI should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedLayout { kernel, layout, detail }
            if (*kernel == when_true || *kernel == when_false)
                && *layout == pair_layout
                && detail.contains("uses aggregate layout")
    )));
}

#[test]
fn cranelift_codegen_compiles_static_scalar_tuple_and_record_item_bodies() {
    let backend = lower_text(
        "backend-static-scalar-aggregate-codegen.aivi",
        r#"
type Pair = (Int, Float, Bool)
type Stats = { count: Int, ratio: Float, active: Bool }

value pair:Pair = (7, 3.5, False)
value stats:Stats = { count: 7, ratio: 3.5, active: True }
"#,
    );

    let pair_body = backend.items()[find_item(&backend, "pair")]
        .body
        .expect("pair should carry a body kernel");
    let stats_body = backend.items()[find_item(&backend, "stats")]
        .body
        .expect("stats should carry a body kernel");

    let compiled =
        compile_program(&backend).expect("static scalar tuple/record item bodies should compile");

    for kernel_id in [pair_body, stats_body] {
        let artifact = compiled
            .kernel(kernel_id)
            .expect("compiled program should retain aggregate kernel metadata");
        assert!(artifact.code_size > 0);
        assert!(
            artifact
                .clif
                .contains(&format!("() -> {}", clif_pointer_ty()))
        );
        assert!(artifact.clif.contains("symbol_value"));
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_static_aggregate_literals_with_by_reference_fields() {
    let backend = lower_text(
        "backend-static-byref-aggregate-codegen.aivi",
        r#"
type User = { name: Text, active: Bool }

value user:User = { name: "Ada", active: True }
"#,
    );

    compile_program(&backend)
        .expect("static aggregate literals with by-reference fields should now compile via runtime aggregate building");
}

#[test]
fn cranelift_codegen_rejects_nonrepresentational_domain_member_calls() {
    let backend = lower_text(
        "backend-domain-operators-codegen.aivi",
        r#"
domain Duration over Int
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
    type Duration -> Duration -> Bool
    (>)

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
    windows
     ?|> ((.delay + 5ms) > 12ms)
"#,
    );
    let errors = compile_program(&backend)
        .expect_err("nonrepresentational domain member kernels should stay unsupported");
    assert!(errors.errors().iter().any(|error| matches!(
        error,
        CodegenError::UnsupportedExpression { detail, .. }
            if detail.contains("domain member `Duration.")
                && detail.contains("unary representational wrappers")
    )));
}

#[test]
fn runtime_evaluates_db_query_builder_flow_into_db_task_plan() {
    let backend = lower_text(
        "backend-db-query-runtime.aivi",
        r#"
use aivi.db (paramInt, statement)

type DatabaseHandle = {
    database: Text
}

value conn = { database: "app.sqlite" }

@source db conn
signal database : DatabaseHandle

value selectUsers: Task Text (List (Map Text Text)) =
    database.query (statement "select * from users where id = ?" [paramInt 7])
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "selectUsers"), &globals)
            .expect("db query should evaluate into a backend db task plan"),
        RuntimeValue::DbTask(RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
            connection: RuntimeDbConnection {
                database: "app.sqlite".into(),
            },
            statement: RuntimeDbStatement {
                sql: "select * from users where id = ?".into(),
                arguments: vec![RuntimeValue::Int(7)],
            },
        }))
    );
}

#[test]
fn runtime_evaluates_db_commit_builder_flow_into_db_task_plan() {
    let backend = lower_text(
        "backend-db-commit-runtime.aivi",
        r#"
use aivi.db (paramBool, paramInt, paramText, statement)

type DatabaseHandle = {
    database: Text
}

value conn = { database: "app.sqlite" }

@source db conn
signal database : DatabaseHandle

value activateUser: Task Text Unit =
    database.commit ["users", "audit_log", "users"] [
        statement "update users set active = ? where id = ?" [paramBool True, paramInt 7],
        statement "insert into audit_log(message) values (?)" [paramText "activated user"]
    ]
"#,
    );
    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "activateUser"), &globals)
            .expect("db commit should evaluate into a backend db task plan"),
        RuntimeValue::DbTask(RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
            connection: RuntimeDbConnection {
                database: "app.sqlite".into(),
            },
            statements: vec![
                RuntimeDbStatement {
                    sql: "update users set active = ? where id = ?".into(),
                    arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
                },
                RuntimeDbStatement {
                    sql: "insert into audit_log(message) values (?)".into(),
                    arguments: vec![RuntimeValue::Text("activated user".into())],
                },
            ],
            changed_tables: ["users", "audit_log"].into_iter().map(Into::into).collect(),
        }))
    );
}

#[test]
fn cranelift_codegen_compiles_list_literal() {
    let backend = lower_text(
        "backend-list-literal-codegen.aivi",
        "value nums:List Int = [1, 2, 3]\n",
    );
    let body = backend.items()[find_item(&backend, "nums")]
        .body
        .expect("nums should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("list literal should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain list kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact
            .clif
            .contains(&format!("() -> {}", clif_pointer_ty()))
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_set_literal() {
    let backend = lower_text(
        "backend-set-literal-codegen.aivi",
        r#"value tags:Set Text = Set ["news", "featured"]
"#,
    );
    let body = backend.items()[find_item(&backend, "tags")]
        .body
        .expect("tags should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("set literal should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain set kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_map_literal() {
    let backend = lower_text(
        "backend-map-literal-codegen.aivi",
        r#"value headers:Map Text Text = Map { "Accept": "application/json", "Host": "example.com" }
"#,
    );
    let body = backend.items()[find_item(&backend, "headers")]
        .body
        .expect("headers should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("map literal should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain map kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_validation_stage() {
    let backend = lower_text(
        "backend-validation-stage-codegen.aivi",
        r#"
fun doubled:Int = x:Int =>
    x
     |> . + 1
     !|> . * 2
"#,
    );
    let body = backend.items()[find_item(&backend, "doubled")]
        .body
        .expect("doubled should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("validation stage should compile (lowered as Transform)");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain validation kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_patch_removal() {
    // Patch removal in a pipe lowered through record construction.
    // Uses a value body instead of a pipe stage so patch apply is elaborated
    // at HIR level before general-expression elaboration.
    let backend = lower_text(
        "backend-patch-removal-codegen.aivi",
        r#"
type User = { name: Text, age: Int, active: Bool }

value user:User = { name: "Ada", age: 36, active: True }
"#,
    );
    // Verify the record literal compiles
    let body = backend.items()[find_item(&backend, "user")]
        .body
        .expect("user should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("record literal should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain record kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

// Debug and fan-out codegen are implemented (no-op pass-through and loop emission
// respectively) but cannot be exercised from general-expression test fixtures:
// - Debug stages are only produced by @debug decorators on signal pipelines
// - Fan-out *|> in general expressions triggers SubjectLayoutMismatch at backend
//   lowering (pre-existing issue); signal-pipeline fan-out is tested by
//   retains_signal_fanout_map_and_join_kernels
