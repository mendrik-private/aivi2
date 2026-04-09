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

