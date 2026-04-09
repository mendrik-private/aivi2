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
fn cranelift_codegen_compiles_bytes_append_intrinsic() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use aivi.core.bytes (
    append
)

fun combine:Bytes = left:Bytes right:Bytes=>    append left right
"#,
    );

    compile_program(&backend).expect("bytes.append should compile with runtime function call");
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

fun wrapPath:Path = raw:Text=>    fromText raw

fun carrierPath:Text = path:Path=>    path.carrier
"#,
    );

    let wrap_path = find_item(&backend, "wrapPath");
    let carrier_path = find_item(&backend, "carrierPath");
    let wrap_body = backend.items()[wrap_path]
        .body
        .expect("wrapPath should carry a body kernel");
    let carrier_body = backend.items()[carrier_path]
        .body
        .expect("carrierPath should carry a body kernel");

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

    let carrier_kernel = &backend.kernels()[carrier_body];
    match &carrier_kernel.exprs()[carrier_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &carrier_kernel.exprs()[*callee].kind,
                KernelExprKind::DomainMember(handle)
                    if handle.domain_name.as_ref() == "Path"
                        && handle.member_name.as_ref() == "carrier"
            ));
        }
        other => {
            panic!("expected carrierPath body to lower into a domain-member apply, found {other:?}")
        }
    }

    let compiled = compile_program(&backend)
        .expect("representational by-reference domain member calls should compile");
    let ptr = clif_pointer_ty();

    for body in [wrap_body, carrier_body] {
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

value home : Path = fromText "/tmp/app"
value raw : Text = home.carrier
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
fn cranelift_codegen_compiles_result_ok() {
    let backend = lower_text(
        "result-ok.aivi",
        r#"
fun make_ok:(Result Text Int) = x:Int =>
    Ok x
"#,
    );
    let body = backend.items()[find_item(&backend, "make_ok")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("Result Ok should compile");
    assert!(compiled.kernel(body).unwrap().code_size > 0);
}

#[test]
fn cranelift_codegen_compiles_result_err() {
    let backend = lower_text(
        "result-err.aivi",
        r#"
fun make_err:(Result Text Int) = msg:Text =>
    Err msg
"#,
    );
    let body = backend.items()[find_item(&backend, "make_err")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("Result Err should compile");
    assert!(compiled.kernel(body).unwrap().code_size > 0);
}

#[test]
fn cranelift_codegen_compiles_validation_valid() {
    let backend = lower_text(
        "validation-valid.aivi",
        r#"
fun make_valid:(Validation Text Int) = x:Int =>
    Valid x
"#,
    );
    let body = backend.items()[find_item(&backend, "make_valid")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("Validation Valid should compile");
    assert!(compiled.kernel(body).unwrap().code_size > 0);
}

#[test]
fn cranelift_codegen_compiles_validation_invalid() {
    let backend = lower_text(
        "validation-invalid.aivi",
        r#"
fun make_invalid:(Validation Text Int) = msg:Text =>
    Invalid msg
"#,
    );
    let body = backend.items()[find_item(&backend, "make_invalid")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("Validation Invalid should compile");
    assert!(compiled.kernel(body).unwrap().code_size > 0);
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
    // The gate predicate involves domain literals (5ms, 12ms) which remain
    // outside the current codegen slice. The (+) and (>) operators themselves
    // now compile for Int-carrier domains via NativeIntBinary, but the literal
    // expressions still produce UnsupportedExpression errors.
    let errors = compile_program(&backend)
        .expect_err("domain literal expressions inside gate predicates should stay unsupported");
    assert!(
        !errors.errors().is_empty(),
        "expected codegen errors for domain literal gate predicate"
    );
}

