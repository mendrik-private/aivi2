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
func extract = d => d.carrier

value raw : Int =
    10ms
     |> extract
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

