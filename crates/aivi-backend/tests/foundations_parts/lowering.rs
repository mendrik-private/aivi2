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
fn lowers_backend_bodies_for_parameterized_from_selectors() {
    let backend = lower_text(
        "backend-parameterized-from-selectors.aivi",
        r#"
type State = { score: Int, ready: Bool }

signal state : Signal State = { score: 0, ready: True }

from state = {
    type Bool
    readyNow: .ready
    type Int -> Bool
    atLeast threshold: .score >= threshold
}

signal currentReady : Signal Bool = readyNow
signal thresholdMet : Signal Bool = atLeast 0
"#,
    );

    let at_least = find_item(&backend, "atLeast");
    let item = &backend.items()[at_least];
    assert_eq!(item.parameters.len(), 1);
    assert!(
        item.body.is_some(),
        "parameterized from-selector should lower a backend body kernel"
    );

    let threshold_met = find_item(&backend, "thresholdMet");
    assert!(
        backend.items()[threshold_met].body.is_some(),
        "signals calling parameterized from-selectors should keep backend bodies"
    );
}

#[test]
fn lowers_backend_bodies_for_parameterized_from_selectors_using_source_helpers() {
    let backend = lower_text(
        "backend-parameterized-from-selectors-helpers.aivi",
        r#"
type State = { score: Int, ready: Bool }

type Int -> State -> Bool
func atLeastFromState = threshold state => state.ready and state.score >= threshold

signal state : Signal State = { score: 0, ready: True }

from state = {
    type Int -> Bool
    atLeast threshold: atLeastFromState threshold
}

signal thresholdMet : Signal Bool = atLeast 0
"#,
    );

    let at_least = find_item(&backend, "atLeast");
    let item = &backend.items()[at_least];
    assert_eq!(item.parameters.len(), 1);
    assert!(
        item.body.is_some(),
        "helper-backed parameterized from-selector should lower a backend body kernel"
    );

    let threshold_met = find_item(&backend, "thresholdMet");
    assert!(
        backend.items()[threshold_met].body.is_some(),
        "signals depending on helper-backed parameterized selectors should keep backend bodies"
    );
}

#[test]
fn lowers_backend_bodies_for_parameterized_from_selectors_using_same_block_signals() {
    let backend = lower_text(
        "backend-parameterized-from-selectors-same-block-signals.aivi",
        r#"
type State = { score: Int, ready: Bool }

signal state : Signal State = { score: 1, ready: True }

from state = {
    score: .score
    ready: .ready

    type Int -> Bool
    atLeast threshold: ready and score >= threshold
}

signal thresholdMet : Signal Bool = atLeast 0
"#,
    );

    let at_least = find_item(&backend, "atLeast");
    let item = &backend.items()[at_least];
    assert_eq!(item.parameters.len(), 1);
    assert!(
        item.body.is_some(),
        "same-block signal-backed parameterized from-selector should lower a backend body kernel"
    );

    let threshold_met = find_item(&backend, "thresholdMet");
    assert!(
        backend.items()[threshold_met].body.is_some(),
        "signals depending on same-block signal-backed parameterized selectors should keep backend bodies"
    );
}

