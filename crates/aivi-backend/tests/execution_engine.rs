use std::collections::BTreeMap;

use aivi_backend::{
    BackendExecutableProgram, BackendExecutionEngine, BackendExecutionEngineKind,
    BackendKernelArtifactCache, ItemKind, KernelEvaluator, NativeKernelArtifactSet,
    NativeKernelPlan, RuntimeBigInt, RuntimeDecimal, RuntimeFloat, RuntimeMap, RuntimeMapEntry,
    RuntimeRecordField, RuntimeValue, compile_native_kernel_artifact, compile_program,
    compute_kernel_fingerprint, decode_native_kernel_artifact_binary,
    encode_native_kernel_artifact_binary, lower_module as lower_backend_module, validate_program,
};
use aivi_base::SourceDatabase;
use aivi_core::{lower_module as lower_core_module, validate_module as validate_core_module};
use aivi_lambda::{lower_module as lower_lambda_module, validate_module as validate_lambda_module};
use aivi_syntax::parse_module;

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

fn find_item(program: &aivi_backend::Program, name: &str) -> aivi_backend::ItemId {
    program
        .items()
        .iter()
        .find(|(_, item)| item.name.as_ref() == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("expected backend item `{name}`"))
}

fn apply_callable_item(
    engine: &mut dyn BackendExecutionEngine,
    program: &aivi_backend::Program,
    item: aivi_backend::ItemId,
    arguments: Vec<RuntimeValue>,
    globals: &BTreeMap<aivi_backend::ItemId, RuntimeValue>,
) -> RuntimeValue {
    let callable = engine
        .evaluate_item(item, globals)
        .expect("engine should evaluate callable items before applying them");
    let kernel = program.items()[item]
        .body
        .expect("callable item should lower into a body kernel");
    engine
        .apply_runtime_callable(kernel, callable, arguments, globals)
        .expect("engine should execute callable items")
}

#[test]
fn kernel_evaluator_supports_the_backend_execution_engine_trait() {
    let backend = lower_text("backend-engine-trait.aivi", "value total:Int = 21 + 21\n");
    let mut engine: Box<dyn BackendExecutionEngine + '_> = Box::new(KernelEvaluator::new(&backend));

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Interpreter);
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "total"), &BTreeMap::new())
            .expect("trait-object execution should evaluate"),
        RuntimeValue::Int(42)
    );
}

#[test]
fn interpreted_executable_program_creates_profiled_jit_engines() {
    let backend = lower_text(
        "backend-engine-profiled.aivi",
        "value total:Int = 21 + 21\n",
    );
    let total = find_item(&backend, "total");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_profiled_engine();

    assert_eq!(executable.engine_kind(), BackendExecutionEngineKind::Jit);
    assert!(executable.compiled_object().is_none());
    assert!(engine.profile().is_some());
    assert_eq!(
        engine
            .evaluate_item(total, &BTreeMap::new())
            .expect("profiled interpreter engine should evaluate"),
        RuntimeValue::Int(42)
    );

    let profile = engine
        .profile_snapshot()
        .expect("profiled interpreter should expose a profile snapshot");
    assert_eq!(profile.items[&total].calls, 1);
}

#[test]
fn compiled_executable_program_keeps_object_artifacts_and_jit_execution() {
    let backend = lower_text(
        "backend-engine-compiled.aivi",
        "value total:Int = 21 + 21\n",
    );
    let total = find_item(&backend, "total");
    let executable = BackendExecutableProgram::compile(&backend)
        .expect("executable-program compile should preserve object emission");
    let compiled = executable
        .compiled_object()
        .expect("compiled executable program should retain object artifacts");
    let mut engine = executable.create_engine();

    assert_eq!(executable.engine_kind(), BackendExecutionEngineKind::Jit);
    assert!(!compiled.object().is_empty());
    assert!(!compiled.kernels().is_empty());
    assert_eq!(
        engine.evaluate_item(total, &BTreeMap::new()).expect(
            "compiled executable program should still evaluate through the lazy JIT engine"
        ),
        RuntimeValue::Int(42)
    );
}

#[test]
fn native_kernel_plan_executes_supported_signal_body_kernels() {
    let backend = lower_text(
        "backend-native-plan-signal-body.aivi",
        r#"
fun increment:Int = value:Int=>    value + 1

signal base = 7
signal next = increment base
"#,
    );
    let next = find_item(&backend, "next");
    let body_kernel = match &backend.items()[next].kind {
        ItemKind::Signal(signal) => signal
            .body_kernel
            .expect("derived signals should lower a dedicated body kernel"),
        other => panic!("expected signal item, found {other:?}"),
    };
    let mut plan = NativeKernelPlan::compile(&backend, body_kernel)
        .expect("supported signal body should compile natively");

    assert_eq!(plan.kernel_id(), body_kernel);
    assert_eq!(
        plan.dependency_layouts(),
        backend.kernels()[body_kernel].environment.as_slice()
    );
    assert_eq!(
        plan.execute(None, &[RuntimeValue::Int(7)], &BTreeMap::new())
            .expect("native plan should execute the signal body"),
        RuntimeValue::Int(8)
    );
}

#[test]
fn executable_program_runs_with_serialized_native_kernel_sidecars() {
    let backend = lower_text(
        "backend-native-sidecar-engine.aivi",
        r#"
fun increment:Int = value:Int=>    value + 1

signal base = 7
signal next = increment base
"#,
    );
    let next = find_item(&backend, "next");
    let body_kernel = match &backend.items()[next].kind {
        ItemKind::Signal(signal) => signal
            .body_kernel
            .expect("derived signals should lower a dedicated body kernel"),
        other => panic!("expected signal item, found {other:?}"),
    };
    let artifact = compile_native_kernel_artifact(&backend, body_kernel)
        .expect("native sidecar compilation should succeed")
        .expect("supported signal body should emit a native sidecar");
    let bytes = encode_native_kernel_artifact_binary(&artifact);
    let decoded =
        decode_native_kernel_artifact_binary(&bytes).expect("encoded native sidecar should decode");
    let mut native_kernels = NativeKernelArtifactSet::default();
    native_kernels.insert(compute_kernel_fingerprint(&backend, body_kernel), decoded);
    let executable =
        BackendExecutableProgram::interpreted(&backend).with_native_kernels(&native_kernels);
    let mut engine = executable.create_engine();

    assert_eq!(
        engine
            .evaluate_signal_body_kernel(body_kernel, &[RuntimeValue::Int(7)], &BTreeMap::new())
            .expect("native sidecar-backed engine should evaluate signal bodies"),
        RuntimeValue::Int(8)
    );
}

#[test]
fn jit_engine_executes_helper_backed_text_kernels() {
    let backend = lower_text(
        "backend-engine-text-jit.aivi",
        r#"
value host:Text = "Ada"
value greeting:Text = "hello {host}"
"#,
    );
    let greeting = find_item(&backend, "greeting");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    assert_eq!(
        engine
            .evaluate_item(greeting, &BTreeMap::new())
            .expect("JIT engine should evaluate helper-backed text interpolation"),
        RuntimeValue::Text("hello Ada".into())
    );
}

#[test]
fn jit_engine_executes_helper_backed_bytes_kernels() {
    let backend = lower_text(
        "backend-engine-bytes-jit.aivi",
        r#"
use aivi.core.bytes (
    append,
    repeat,
    slice,
    toText
)

fun makeBlob:Bytes = seed:Int=>
    append (repeat seed 1) (slice 1 3 (repeat (seed + 1) 4))

fun decodeBlob:(Option Text) = seed:Int=>
    toText (makeBlob seed)
"#,
    );
    let make_blob = find_item(&backend, "makeBlob");
    let decode_blob = find_item(&backend, "decodeBlob");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            make_blob,
            vec![RuntimeValue::Int(65)],
            &BTreeMap::new()
        ),
        RuntimeValue::Bytes(b"ABB".to_vec().into())
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            decode_blob,
            vec![RuntimeValue::Int(65)],
            &BTreeMap::new()
        ),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Text("ABB".into())))
    );
}

#[test]
fn jit_engine_executes_inline_scalar_option_kernels() {
    let backend = lower_text(
        "backend-engine-inline-option-jit.aivi",
        r#"
fun passMaybeInt:(Option Int) = value:(Option Int)=>    value
fun passMaybeFloat:(Option Float) = value:(Option Float)=>    value
fun passMaybeBool:(Option Bool) = value:(Option Bool)=>    value
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();
    let globals = BTreeMap::new();
    let option_float = RuntimeValue::OptionSome(Box::new(RuntimeValue::Float(
        RuntimeFloat::new(3.5).expect("finite float should build a runtime float"),
    )));

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "passMaybeInt"),
            vec![RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(-7)))],
            &globals
        ),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(-7)))
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "passMaybeInt"),
            vec![RuntimeValue::OptionNone],
            &globals
        ),
        RuntimeValue::OptionNone
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "passMaybeFloat"),
            vec![option_float.clone()],
            &globals
        ),
        option_float
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "passMaybeBool"),
            vec![RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(
                false
            )))],
            &globals
        ),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(false)))
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "passMaybeBool"),
            vec![RuntimeValue::OptionNone],
            &globals
        ),
        RuntimeValue::OptionNone
    );
}

#[test]
fn jit_engine_executes_collection_kernels() {
    let backend = lower_text(
        "backend-engine-collections-jit.aivi",
        r#"
value contacts:List { id: Int, name: Text } = [
    { id: 1, name: "Ada" },
    { id: 2, name: "Grace" }
]

fun keepContacts:List { id: Int, name: Text } = contacts:List { id: Int, name: Text } =>
    contacts

value headers:Map Text Text =
    Map {
        "Authorization": "Bearer demo",
        "Accept": "application/json"
    }

fun keepHeaders:Map Text Text = input:Map Text Text =>
    input

value tags:Set Text =
    Set [
        "news",
        "featured"
    ]

fun keepTags:Set Text = input:Set Text =>
    input
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();
    let globals = BTreeMap::new();
    let contacts = RuntimeValue::List(vec![
        RuntimeValue::Record(vec![
            RuntimeRecordField {
                label: "id".into(),
                value: RuntimeValue::Int(1),
            },
            RuntimeRecordField {
                label: "name".into(),
                value: RuntimeValue::Text("Ada".into()),
            },
        ]),
        RuntimeValue::Record(vec![
            RuntimeRecordField {
                label: "id".into(),
                value: RuntimeValue::Int(2),
            },
            RuntimeRecordField {
                label: "name".into(),
                value: RuntimeValue::Text("Grace".into()),
            },
        ]),
    ]);
    let headers = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("Authorization".into()),
            value: RuntimeValue::Text("Bearer demo".into()),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("Accept".into()),
            value: RuntimeValue::Text("application/json".into()),
        },
    ]));
    let tags = RuntimeValue::Set(vec![
        RuntimeValue::Text("news".into()),
        RuntimeValue::Text("featured".into()),
    ]);

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "contacts"), &globals)
            .expect("JIT engine should evaluate list-of-record items"),
        contacts.clone()
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "keepContacts"),
            vec![contacts.clone()],
            &globals
        ),
        contacts
    );
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "headers"), &globals)
            .expect("JIT engine should evaluate map items"),
        headers.clone()
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "keepHeaders"),
            vec![headers.clone()],
            &globals
        ),
        headers
    );
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "tags"), &globals)
            .expect("JIT engine should evaluate set items"),
        tags.clone()
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "keepTags"),
            vec![tags.clone()],
            &globals
        ),
        tags
    );
}

#[test]
fn jit_engine_executes_decimal_and_bigint_kernels() {
    let backend = lower_text(
        "backend-engine-numerics-jit.aivi",
        r#"
value decimalBase:Decimal = 19.25d
value bigintBase:BigInt = 123456789012345678901234567890n

fun addDecimals:Decimal = left:Decimal right:Decimal =>
    left + right

fun decimalGt:Bool = left:Decimal right:Decimal =>
    left > right

fun addBigints:BigInt = left:BigInt right:BigInt =>
    left + right

fun bigintGt:Bool = left:BigInt right:BigInt =>
    left > right
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();
    let globals = BTreeMap::new();
    let left_decimal = RuntimeValue::Decimal(
        RuntimeDecimal::parse_literal("19.25d").expect("decimal should parse"),
    );
    let right_decimal = RuntimeValue::Decimal(
        RuntimeDecimal::parse_literal("0.75d").expect("decimal should parse"),
    );
    let left_bigint = RuntimeValue::BigInt(
        RuntimeBigInt::parse_literal("123456789012345678901234567890n")
            .expect("bigint should parse"),
    );
    let right_bigint =
        RuntimeValue::BigInt(RuntimeBigInt::parse_literal("10n").expect("bigint should parse"));

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "decimalBase"), &globals)
            .expect("JIT engine should evaluate decimal literal items"),
        left_decimal.clone()
    );
    assert_eq!(
        engine
            .evaluate_item(find_item(&backend, "bigintBase"), &globals)
            .expect("JIT engine should evaluate bigint literal items"),
        left_bigint.clone()
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "addDecimals"),
            vec![left_decimal.clone(), right_decimal.clone()],
            &globals
        ),
        RuntimeValue::Decimal(
            RuntimeDecimal::parse_literal("20.00d").expect("decimal should parse")
        )
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "decimalGt"),
            vec![left_decimal.clone(), right_decimal],
            &globals
        ),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "addBigints"),
            vec![left_bigint.clone(), right_bigint.clone()],
            &globals
        ),
        RuntimeValue::BigInt(
            RuntimeBigInt::parse_literal("123456789012345678901234567900n")
                .expect("bigint should parse")
        )
    );
    assert_eq!(
        apply_callable_item(
            engine.as_mut(),
            &backend,
            find_item(&backend, "bigintGt"),
            vec![left_bigint, right_bigint],
            &globals
        ),
        RuntimeValue::Bool(true)
    );
}

#[test]
fn jit_engine_supports_opaque_matrix_layouts() {
    let backend = lower_text(
        "backend-engine-matrix.aivi",
        r#"
use aivi.matrix (
    Matrix,
    MatrixError
)

value matrix:Matrix Int =
    MkMatrix 2 2 [
        [1, 2],
        [3, 4]
    ]

value matrixResult:Result MatrixError (Matrix Int) =
    Ok matrix

value matrixWidth:Int =
    matrix
    ||> MkMatrix w _ _ -> w

value matrixHeight:Int =
    matrix
    ||> MkMatrix _ h _ -> h

value matrixRows:List (List Int) =
    matrix
    ||> MkMatrix _ _ inputRows -> inputRows
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut engine = executable.create_engine();
    let mut interpreter = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    assert_eq!(engine.kind(), BackendExecutionEngineKind::Jit);
    for name in [
        "matrix",
        "matrixResult",
        "matrixWidth",
        "matrixHeight",
        "matrixRows",
    ] {
        let item = find_item(&backend, name);
        assert_eq!(
            engine
                .evaluate_item(item, &globals)
                .unwrap_or_else(|_| panic!("JIT engine should evaluate `{name}`")),
            interpreter
                .evaluate_item(item, &globals)
                .unwrap_or_else(|_| panic!("interpreter should evaluate `{name}`"))
        );
    }
}

#[test]
fn jit_engine_supports_imported_generic_matrix_helpers() {
    let backend = lower_text(
        "backend-engine-matrix-generic.aivi",
        r#"
use aivi.matrix (
    Matrix,
    MatrixError,
    fromRows,
    height,
    rows,
    width
)

value matrixResult:Result MatrixError (Matrix Int) =
    fromRows [
        [1, 2],
        [3, 4]
    ]

value liftedMatrixResult:Result MatrixError (Matrix Int) =
    matrixResult

value matrixIsOk:Int =
    fromRows [
        [1, 2],
        [3, 4]
    ]
    ||> Ok _ -> 1
    ||> Err _ -> -1

value liftedResultIsOk:Int =
    matrixResult
    ||> Ok _ -> 1
    ||> Err _ -> -1

value matrixWidth:Int =
    fromRows [
        [1, 2],
        [3, 4]
    ]
    ||> Ok matrix -> width matrix
    ||> Err _ -> 0

value matrixHeight:Int =
    fromRows [
        [1, 2],
        [3, 4]
    ]
    ||> Ok matrix -> height matrix
    ||> Err _ -> 0

value matrixRows:List (List Int) =
    fromRows [
        [1, 2],
        [3, 4]
    ]
    ||> Ok matrix -> rows matrix
    ||> Err _ -> []
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut jit = executable.create_engine();
    let mut interpreter = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();

    for name in [
        "matrixResult",
        "liftedMatrixResult",
        "matrixIsOk",
        "liftedResultIsOk",
        "matrixWidth",
        "matrixHeight",
        "matrixRows",
    ] {
        let item = find_item(&backend, name);
        assert_eq!(
            jit.evaluate_item(item, &globals)
                .unwrap_or_else(|_| panic!("JIT engine should evaluate `{name}`")),
            interpreter
                .evaluate_item(item, &globals)
                .unwrap_or_else(|_| panic!("interpreter should evaluate `{name}`"))
        );
    }
}

#[test]
fn jit_engine_matches_interpreter_results_for_supported_and_fallback_items() {
    let backend = lower_text(
        "backend-engine-parity.aivi",
        r#"
use aivi.core.bytes (
    append,
    repeat,
    slice,
    toText
)

value host:Text = "Ada"
value greeting:Text = "hello {host}"
fun makeBlob:Bytes = seed:Int=>    append (repeat seed 1) (slice 1 3 (repeat (seed + 1) 4))
fun decodeBlob:(Option Text) = seed:Int=>    toText (makeBlob seed)
fun passMaybeInt:(Option Int) = value:(Option Int)=>    value
fun passMaybeFloat:(Option Float) = value:(Option Float)=>    value
fun passMaybeBool:(Option Bool) = value:(Option Bool)=>    value
value contacts:List { id: Int, name: Text } = [
    { id: 1, name: "Ada" },
    { id: 2, name: "Grace" }
]
fun keepContacts:List { id: Int, name: Text } = contacts:List { id: Int, name: Text } =>
    contacts
value headers:Map Text Text =
    Map {
        "Authorization": "Bearer demo",
        "Accept": "application/json"
    }
fun keepHeaders:Map Text Text = input:Map Text Text =>    input
value tags:Set Text =
    Set [
        "news",
        "featured"
    ]
fun keepTags:Set Text = input:Set Text =>    input
value decimalBase:Decimal = 19.25d
value bigintBase:BigInt = 123456789012345678901234567890n
fun addDecimals:Decimal = left:Decimal right:Decimal =>    left + right
fun decimalGt:Bool = left:Decimal right:Decimal =>    left > right
fun addBigints:BigInt = left:BigInt right:BigInt =>    left + right
fun bigintGt:Bool = left:BigInt right:BigInt =>    left > right
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut jit = executable.create_engine();
    let mut interpreter = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    let option_float = RuntimeValue::OptionSome(Box::new(RuntimeValue::Float(
        RuntimeFloat::new(3.5).expect("finite float should build a runtime float"),
    )));
    let contacts = RuntimeValue::List(vec![
        RuntimeValue::Record(vec![
            RuntimeRecordField {
                label: "id".into(),
                value: RuntimeValue::Int(1),
            },
            RuntimeRecordField {
                label: "name".into(),
                value: RuntimeValue::Text("Ada".into()),
            },
        ]),
        RuntimeValue::Record(vec![
            RuntimeRecordField {
                label: "id".into(),
                value: RuntimeValue::Int(2),
            },
            RuntimeRecordField {
                label: "name".into(),
                value: RuntimeValue::Text("Grace".into()),
            },
        ]),
    ]);
    let headers = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("Authorization".into()),
            value: RuntimeValue::Text("Bearer demo".into()),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("Accept".into()),
            value: RuntimeValue::Text("application/json".into()),
        },
    ]));
    let tags = RuntimeValue::Set(vec![
        RuntimeValue::Text("news".into()),
        RuntimeValue::Text("featured".into()),
    ]);

    assert_eq!(jit.kind(), BackendExecutionEngineKind::Jit);
    for item in [
        "greeting",
        "contacts",
        "headers",
        "tags",
        "decimalBase",
        "bigintBase",
    ] {
        let item_id = find_item(&backend, item);
        assert_eq!(
            jit.evaluate_item(item_id, &globals)
                .expect("JIT engine should evaluate the item"),
            interpreter
                .evaluate_item(item_id, &globals)
                .expect("interpreter should evaluate the same item")
        );
    }
    for item in ["makeBlob", "decodeBlob"] {
        let item_id = find_item(&backend, item);
        let kernel = backend.items()[item_id]
            .body
            .expect("callable item should lower into a body kernel");
        let jit_callable = jit
            .evaluate_item(item_id, &globals)
            .expect("JIT engine should evaluate the callable item");
        let interpreter_callable = interpreter
            .evaluate_item(item_id, &globals)
            .expect("interpreter should evaluate the callable item");
        assert_eq!(
            jit.apply_runtime_callable(kernel, jit_callable, vec![RuntimeValue::Int(65)], &globals)
                .expect("JIT engine should execute the callable item"),
            interpreter
                .apply_runtime_callable(
                    kernel,
                    interpreter_callable,
                    vec![RuntimeValue::Int(65)],
                    &globals
                )
                .expect("interpreter should execute the same callable item")
        );
    }
    for (item, argument) in [
        ("keepContacts", contacts),
        ("keepHeaders", headers),
        ("keepTags", tags),
    ] {
        let item_id = find_item(&backend, item);
        let arguments = vec![argument];
        assert_eq!(
            apply_callable_item(jit.as_mut(), &backend, item_id, arguments.clone(), &globals),
            apply_callable_item(&mut interpreter, &backend, item_id, arguments, &globals)
        );
    }
    for (item, argument) in [
        (
            "passMaybeInt",
            RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(-7))),
        ),
        ("passMaybeInt", RuntimeValue::OptionNone),
        ("passMaybeFloat", option_float.clone()),
        (
            "passMaybeBool",
            RuntimeValue::OptionSome(Box::new(RuntimeValue::Bool(false))),
        ),
        ("passMaybeBool", RuntimeValue::OptionNone),
    ] {
        let item_id = find_item(&backend, item);
        assert_eq!(
            apply_callable_item(
                jit.as_mut(),
                &backend,
                item_id,
                vec![argument.clone()],
                &globals
            ),
            apply_callable_item(
                &mut interpreter,
                &backend,
                item_id,
                vec![argument],
                &globals
            )
        );
    }
}

#[test]
fn jit_engine_falls_back_to_interpreter_for_inline_text_constructor_patterns() {
    let backend = lower_text(
        "backend-engine-inline-text-patterns.aivi",
        r#"
type Key =
  | Key Text

type Direction =
  | Up
  | Down

fun arrowKey:(Option Direction) = key:Key=>    key
     ||> Key "ArrowUp"   -> Some Up
     ||> Key "ArrowDown" -> Some Down
     ||> _               -> None

value isDown:Bool = arrowKey (Key "ArrowDown")
     ||> Some Down -> True
     ||> _         -> False
"#,
    );
    let executable = BackendExecutableProgram::interpreted(&backend);
    let mut jit = executable.create_engine();
    let mut interpreter = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    let item = find_item(&backend, "isDown");

    assert_eq!(
        jit.evaluate_item(item, &globals)
            .expect("lazy JIT should evaluate text-pattern items via fallback"),
        interpreter
            .evaluate_item(item, &globals)
            .expect("interpreter should evaluate text-pattern items")
    );
    assert_eq!(
        interpreter
            .evaluate_item(item, &globals)
            .expect("interpreter should evaluate the same item"),
        RuntimeValue::Bool(true)
    );
}

#[test]
fn kernel_fingerprints_stay_stable_for_unchanged_kernels() {
    let original = lower_text(
        "backend-engine-fingerprint.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 1 + 1\n",
    );
    let changed = lower_text(
        "backend-engine-fingerprint.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 2 + 2\n",
    );

    let original_total = original.items()[find_item(&original, "total")]
        .body
        .expect("total should lower into a body kernel");
    let original_other = original.items()[find_item(&original, "other")]
        .body
        .expect("other should lower into a body kernel");
    let changed_total = changed.items()[find_item(&changed, "total")]
        .body
        .expect("total should lower into a body kernel");
    let changed_other = changed.items()[find_item(&changed, "other")]
        .body
        .expect("other should lower into a body kernel");

    let original_exec = BackendExecutableProgram::interpreted(&original);
    let changed_exec = BackendExecutableProgram::interpreted(&changed);

    assert_eq!(
        original_exec.kernel_fingerprint(original_total),
        changed_exec.kernel_fingerprint(changed_total)
    );
    assert_ne!(
        original_exec.kernel_fingerprint(original_other),
        changed_exec.kernel_fingerprint(changed_other)
    );
}

#[test]
fn lazy_kernel_compilation_reuses_eager_kernel_metadata_for_supported_programs() {
    let backend = lower_text(
        "backend-engine-lazy-supported.aivi",
        "value total:Int = 21 + 21\nvalue other:Int = 5 + 8\n",
    );
    let total = backend.items()[find_item(&backend, "total")]
        .body
        .expect("total should lower into a body kernel");

    let eager = compile_program(&backend).expect("full-program compilation should succeed");
    let executable = BackendExecutableProgram::interpreted(&backend);
    let lazy = executable
        .compile_kernel(total)
        .expect("single-kernel lazy compilation should succeed");

    assert_eq!(
        lazy.metadata(),
        eager
            .kernel(total)
            .expect("eager compilation should retain the same kernel metadata")
    );
    assert!(!lazy.object().is_empty());
}

#[test]
fn lazy_kernel_compilation_can_skip_unrelated_unsupported_kernels_and_reuse_memory_cache() {
    let backend = lower_text(
        "backend-engine-lazy-unsupported.aivi",
        r#"
domain Path over Text

fun samePath:Bool = left:Path right:Path=>    left == right
value total:Int = 21 + 21
"#,
    );
    let total = backend.items()[find_item(&backend, "total")]
        .body
        .expect("total should lower into a body kernel");

    assert!(
        compile_program(&backend).is_err(),
        "full-program eager compilation should still reject unrelated unsupported kernels"
    );

    let executable = BackendExecutableProgram::interpreted(&backend);
    let expected_fingerprint = executable.kernel_fingerprint(total);
    let lazy = executable
        .compile_kernel(total)
        .expect("single-kernel lazy compilation should still compile supported kernels");
    assert_eq!(lazy.kernel_id(), total);
    assert_eq!(lazy.fingerprint(), expected_fingerprint);
    assert!(!lazy.object().is_empty());

    let mut cache = BackendKernelArtifactCache::new();
    let cached = cache
        .get_or_compile(&backend, total)
        .expect("memory cache should compile and store the first lazy artifact")
        .clone();
    let cached_again = cache
        .get_or_compile(&backend, total)
        .expect("memory cache should reuse the stored lazy artifact")
        .clone();
    assert_eq!(cache.len(), 1);
    assert_eq!(cached, cached_again);
    assert_eq!(cached, lazy);
}
