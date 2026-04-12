#[test]
fn lowers_ord_backed_domain_comparisons_into_backend_gate_kernels() {
    let backend = lower_text(
        "backend-ord-domain-operators.aivi",
        r#"
domain Duration over Int
    suffix ms : Int = value => Duration value
    type Duration -> Int
    toMillis value = value

instance Eq Duration = {
    (==) left right = toMillis left == toMillis right
    (!=) left right = toMillis left != toMillis right
}

instance Ord Duration = {
    compare left right = compare (toMillis left) (toMillis right)
}

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
    windows
     ?|> (.delay > 12ms)
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
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(
                    crate::BuiltinClassMemberIntrinsic::StructuralEq,
                )) => {}
                other => panic!(
                    "expected outer comparison to lower through structural equality, found {other:?}"
                ),
            }
            match &predicate_kernel.exprs()[arguments[0]].kind {
                KernelExprKind::Apply {
                    callee,
                    arguments: compare_args,
                } => {
                    assert_eq!(compare_args.len(), 2);
                    match &predicate_kernel.exprs()[*callee].kind {
                        KernelExprKind::ExecutableEvidence(
                            ExecutableEvidence::Authored(_),
                        ) => {}
                        KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(
                            crate::BuiltinClassMemberIntrinsic::Compare { .. },
                        )) => {}
                        other => panic!(
                            "expected Ord-backed inner compare callee, found {other:?}"
                        ),
                    }
                    assert!(matches!(
                        &predicate_kernel.exprs()[compare_args[0]].kind,
                        KernelExprKind::Projection { .. }
                    ));
                    match &predicate_kernel.exprs()[compare_args[1]].kind {
                        KernelExprKind::Apply {
                            callee,
                            arguments: suffix_args,
                        } => {
                            assert_eq!(suffix_args.len(), 1);
                            assert!(matches!(
                                &predicate_kernel.exprs()[suffix_args[0]].kind,
                                KernelExprKind::Integer(_)
                            ));
                            match &predicate_kernel.exprs()[*callee].kind {
                                KernelExprKind::Item(item_id) => {
                                    assert!(backend.items()[*item_id].name.ends_with("::ms"));
                                }
                                other => panic!(
                                    "expected comparison suffix operand to lower as an item apply, found {other:?}"
                                ),
                            }
                        }
                        other => panic!(
                            "expected comparison suffix operand to lower as a constructor call, found {other:?}"
                        ),
                    }
                }
                other => panic!(
                    "expected outer comparison to apply a builtin compare result, found {other:?}"
                ),
            }
            match &predicate_kernel.exprs()[arguments[1]].kind {
                KernelExprKind::SumConstructor(handle) => {
                    assert_eq!(handle.type_name.as_ref(), "Ordering");
                    assert_eq!(handle.variant_name.as_ref(), "Greater");
                    assert_eq!(handle.field_count, 0);
                }
                other => panic!(
                    "expected outer comparison to test against Ordering.Greater, found {other:?}"
                ),
            }
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
        RuntimeValue::Int(15)
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
    suffix ms : Int = value => Duration value
    type Duration -> Duration -> Duration
    (+)
    (+) = left right => left + right

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

    let parameterized = lower_text(
        "backend-parameterized-domain-operator-runtime.aivi",
        r#"
domain Amount A over A
    wrap : A -> Amount A
    toValue : Amount A -> A
    toValue = amount:A => amount

value total : Amount Int = wrap 3
value raw : Int = toValue total
"#,
    );
    let mut evaluator = KernelEvaluator::new(&parameterized);
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&parameterized, "total"), &BTreeMap::new())
            .expect("parameterized domain value should evaluate"),
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
    suffix ms : Int = value => Duration value
    type Duration -> Int
    extract duration = duration

type Duration -> Int
func unwrap = duration => extract duration

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
    extract duration = duration

value raw : Int = extract (make 10)
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
fn runtime_evaluates_ord_backed_domain_gate_predicates() {
    let backend = lower_text(
        "backend-ord-domain-operators-runtime.aivi",
        r#"
domain Duration over Int
    suffix ms : Int = value => Duration value
    type Duration -> Int
    toMillis value = value

instance Eq Duration = {
    (==) left right = toMillis left == toMillis right
    (!=) left right = toMillis left != toMillis right
}

instance Ord Duration = {
    compare left right = compare (toMillis left) (toMillis right)
}

type Window = {
    delay: Duration
}

signal windows : Signal Window = { delay: 10ms }

signal slowWindows : Signal Window =
    windows
     ?|> (.delay > 12ms)
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
        value: RuntimeValue::Int(15),
    }]);
    let faster = RuntimeValue::Record(vec![RuntimeRecordField {
        label: "delay".into(),
        value: RuntimeValue::Int(6),
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
