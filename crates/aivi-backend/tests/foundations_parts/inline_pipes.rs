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

