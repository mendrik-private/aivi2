#[test]
fn cranelift_codegen_compiles_constructor_pattern_with_sum_variants() {
    let backend = lower_text(
        "backend-constructor-pattern-codegen.aivi",
        r#"
type Shape =
  | Circle Int
  | Rect Int Int
  | Point

type Shape -> Text
func label = s => s
 ||> Circle _ -> "circle"
 ||> Rect _ _ -> "rect"
 ||> Point -> "point"

value circleLabel = label (Circle 5)
value rectLabel = label (Rect 3 4)
value pointLabel = label Point
"#,
    );

    let body = backend.items()[find_item(&backend, "label")]
        .body
        .expect("label should carry a body kernel");
    let compiled = compile_program(&backend).expect("constructor pattern codegen should succeed");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain constructor pattern kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact.clif.contains("icmp_imm eq"),
        "sum pattern should emit tag comparison; CLIF was:\n{}",
        artifact.clif
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_record_destructuring_pattern() {
    let backend = lower_text(
        "backend-record-pattern-codegen.aivi",
        r#"
type User = { name: Text, active: Bool }

type User -> Text
func greeting = u => u
 ||> { name, active: True } -> name
 ||> { active: False } -> "inactive"

value result = greeting { name: "Ada", active: True }
"#,
    );

    let body = backend.items()[find_item(&backend, "greeting")]
        .body
        .expect("greeting should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("record destructuring pattern codegen should succeed");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain record pattern kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_nested_constructor_pattern() {
    let backend = lower_text(
        "backend-nested-pattern-codegen.aivi",
        r#"
type Role =
  | Admin
  | Guest

type Profile = { name: Text, role: Role }

type Profile -> Text
func describe = p => p
 ||> { role: Admin } -> "admin"
 ||> { role: Guest } -> "guest"

value adminDesc = describe { name: "Ada", role: Admin }
value guestDesc = describe { name: "Bob", role: Guest }
"#,
    );

    let body = backend.items()[find_item(&backend, "describe")]
        .body
        .expect("describe should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("nested constructor pattern codegen should succeed");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain nested pattern kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_list_pattern_with_length_discrimination() {
    let backend = lower_text(
        "backend-list-pattern-codegen.aivi",
        r#"
type List Int -> Text
func classify = xs => xs
 ||> [] -> "empty"
 ||> [_] -> "one"
 ||> _ -> "many"

value emptyLabel = classify []
value oneLabel = classify [7]
value manyLabel = classify [1, 2, 3]
"#,
    );

    let body = backend.items()[find_item(&backend, "classify")]
        .body
        .expect("classify should carry a body kernel");
    let compiled = compile_program(&backend).expect("list pattern codegen should succeed");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain list pattern kernel metadata");
    assert!(artifact.code_size > 0);
    // List patterns emit icmp_imm eq for length discrimination (e.g. == 0 for [],
    // == 1 for [_]). The aivi_list_len calls appear as fn references in CLIF.
    assert!(
        artifact.clif.contains("icmp_imm eq") && artifact.clif.contains("call fn"),
        "list pattern should emit length checks via list_len calls; CLIF was:\n{}",
        artifact.clif
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_wildcard_and_integer_patterns() {
    let backend = lower_text(
        "backend-wildcard-int-pattern-codegen.aivi",
        r#"
type Int -> Text
func describeN = n => n
 ||> 0 -> "zero"
 ||> 1 -> "one"
 ||> _ -> "other"

value zeroLabel = describeN 0
value oneLabel = describeN 1
value otherLabel = describeN 42
"#,
    );

    let body = backend.items()[find_item(&backend, "describeN")]
        .body
        .expect("describeN should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("wildcard and integer pattern codegen should succeed");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain integer pattern kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(
        artifact.clif.contains("icmp_imm eq"),
        "integer pattern should emit icmp_imm eq; CLIF was:\n{}",
        artifact.clif
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_domain_binary_arithmetic_operators() {
    let backend = lower_text(
        "domain-binary-arithmetic.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
    type Duration -> Duration -> Duration
    (-)
    type Duration -> Duration -> Duration
    (*)
    type Duration -> Duration -> Duration
    (/)
    type Duration -> Duration -> Duration
    (%)
}

fun add_durations:Duration = a:Duration b:Duration=>    a + b

fun sub_durations:Duration = a:Duration b:Duration=>    a - b

fun mul_durations:Duration = a:Duration b:Duration=>    a * b

fun div_durations:Duration = a:Duration b:Duration=>    a / b

fun mod_durations:Duration = a:Duration b:Duration=>    a % b
"#,
    );

    let compiled = compile_program(&backend).expect("domain binary arithmetic should compile");
    let ptr = clif_pointer_ty();

    for name in [
        "add_durations",
        "sub_durations",
        "mul_durations",
        "div_durations",
        "mod_durations",
    ] {
        let item = find_item(&backend, name);
        let body = backend.items()[item]
            .body
            .expect("arithmetic function should carry a body kernel");
        let artifact = compiled
            .kernel(body)
            .expect("compiled program should retain domain arithmetic kernel metadata");
        assert!(
            artifact.code_size > 0,
            "{name} should produce non-empty native code"
        );
        assert!(
            artifact.clif.contains(&format!("({ptr}, {ptr}) -> {ptr}")),
            "{name} CLIF should have (ptr, ptr) -> ptr signature, got:\n{}",
            artifact.clif
        );
        assert!(
            !artifact.clif.contains("call"),
            "{name} CLIF should not contain function calls, got:\n{}",
            artifact.clif
        );
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_domain_binary_comparison_operators() {
    let backend = lower_text(
        "domain-binary-comparison.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
    type Duration -> Duration -> Bool
    (>)
}

fun gt_durations:Bool = a:Duration b:Duration=>    a > b
"#,
    );

    let compiled = compile_program(&backend).expect("domain binary comparison should compile");
    let ptr = clif_pointer_ty();

    let item = find_item(&backend, "gt_durations");
    let body = backend.items()[item]
        .body
        .expect("comparison function should carry a body kernel");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain domain comparison kernel metadata");
    assert!(
        artifact.code_size > 0,
        "gt_durations should produce non-empty native code"
    );
    assert!(
        artifact.clif.contains(&format!("({ptr}, {ptr}) -> i8")),
        "gt_durations CLIF should have (ptr, ptr) -> i8 signature, got:\n{}",
        artifact.clif
    );
    assert!(
        !artifact.clif.contains("call"),
        "gt_durations CLIF should not contain function calls, got:\n{}",
        artifact.clif
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_domain_carrier_accessor() {
    let backend = lower_text(
        "domain-carrier-accessor.aivi",
        r#"
domain Duration over Int = {
    literal ms : Int -> Duration
    type Duration -> Duration -> Duration
    (+)
}

type Duration -> Int
func unwrap_duration = d => d.carrier
"#,
    );

    let compiled = compile_program(&backend).expect("domain carrier accessor should compile");
    let ptr = clif_pointer_ty();

    let item = find_item(&backend, "unwrap_duration");
    let body = backend.items()[item]
        .body
        .expect("carrier accessor function should carry a body kernel");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain domain carrier kernel metadata");
    assert!(
        artifact.code_size > 0,
        "unwrap_duration should produce non-empty native code"
    );
    assert!(
        artifact.clif.contains(&format!("({ptr}) -> {ptr}")),
        "unwrap_duration CLIF should have (ptr) -> ptr signature, got:\n{}",
        artifact.clif
    );
    assert!(
        !artifact.clif.contains("call"),
        "unwrap_duration CLIF should not contain function calls (identity), got:\n{}",
        artifact.clif
    );
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_recurrence_kernels() {
    let backend = lower_text(
        "recurrence-codegen.aivi",
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

    let retried = find_item(&backend, "retried");
    let retried_recurrence = backend.pipelines()[first_pipeline(&backend, retried)]
        .recurrence
        .as_ref()
        .expect("retried should carry a recurrence plan");

    let compiled = compile_program(&backend).expect("recurrence codegen should succeed");

    // Verify seed kernels compile
    let polled_seed = compiled
        .kernel(polled_recurrence.seed)
        .expect("polled recurrence seed should compile");
    assert!(polled_seed.code_size > 0);
    assert!(polled_seed.symbol.contains("recurrence_seed"));

    let retried_seed = compiled
        .kernel(retried_recurrence.seed)
        .expect("retried recurrence seed should compile");
    assert!(retried_seed.code_size > 0);
    assert!(retried_seed.symbol.contains("recurrence_seed"));

    // Verify start-stage kernels compile
    let polled_start = compiled
        .kernel(polled_recurrence.start.kernel)
        .expect("polled recurrence start should compile");
    assert!(polled_start.code_size > 0);
    assert!(polled_start.symbol.contains("recurrence_start"));

    let retried_start = compiled
        .kernel(retried_recurrence.start.kernel)
        .expect("retried recurrence start should compile");
    assert!(retried_start.code_size > 0);
    assert!(retried_start.symbol.contains("recurrence_start"));

    // Verify step-stage kernels compile
    for (label, step) in polled_recurrence
        .steps
        .iter()
        .map(|s| ("polled", s))
        .chain(retried_recurrence.steps.iter().map(|s| ("retried", s)))
    {
        let artifact = compiled
            .kernel(step.kernel)
            .unwrap_or_else(|| panic!("{label} recurrence step should compile"));
        assert!(artifact.code_size > 0);
        assert!(artifact.symbol.contains("recurrence_step"));
    }

    // Verify wakeup-witness kernels compile (representational domain literals)
    let polled_witness = polled_recurrence
        .non_source_wakeup
        .as_ref()
        .expect("polled should have a wakeup witness");
    let polled_witness_artifact = compiled
        .kernel(polled_witness.kernel)
        .expect("polled wakeup witness should compile");
    assert!(polled_witness_artifact.code_size > 0);
    assert!(
        polled_witness_artifact
            .symbol
            .contains("recurrence_witness")
    );

    let retried_witness = retried_recurrence
        .non_source_wakeup
        .as_ref()
        .expect("retried should have a wakeup witness");
    let retried_witness_artifact = compiled
        .kernel(retried_witness.kernel)
        .expect("retried wakeup witness should compile");
    assert!(retried_witness_artifact.code_size > 0);
    assert!(
        retried_witness_artifact
            .symbol
            .contains("recurrence_witness")
    );

    assert!(!compiled.object().is_empty());
}
