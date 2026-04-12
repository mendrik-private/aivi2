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
    let compiled = compile_program(&backend).expect("list literal should compile");
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
    let compiled = compile_program(&backend).expect("set literal should compile");
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
    let compiled = compile_program(&backend).expect("map literal should compile");
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
fun step1:Result Text Int = value:Int => Ok (value + 1)
fun step2:Result Text Int = value:Int => Ok (value * 2)
fun doubled:Result Text Int = x:Int =>
    x
     !|> step1
     !|> step2
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
fn cranelift_codegen_compiles_patch_replace() {
    let backend = lower_text(
        "backend-patch-replace-codegen.aivi",
        r#"
type User = { name: Text, age: Int, active: Bool }

fun patch_user:User = u:User =>
    u <| { age: 42 }
"#,
    );
    let body = backend.items()[find_item(&backend, "patch_user")]
        .body
        .expect("patch_user should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("patch replace should compile (desugared to record)");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain patch kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_patch_removal() {
    let backend = lower_text(
        "backend-patch-removal-codegen.aivi",
        r#"
type User = { name: Text, age: Int, active: Bool }

fun strip_active:{ name: Text, age: Int } = u:User =>
    u <| { active: - }
"#,
    );
    let body = backend.items()[find_item(&backend, "strip_active")]
        .body
        .expect("strip_active should carry a body kernel");
    let compiled =
        compile_program(&backend).expect("patch removal should compile (desugared to record)");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain patch-removal kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_decimal_arithmetic() {
    let backend = lower_text(
        "decimal-arith.aivi",
        r#"
fun add_decimals:Decimal = a:Decimal b:Decimal =>
    a + b
"#,
    );
    let body = backend.items()[find_item(&backend, "add_decimals")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("decimal arithmetic should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain decimal add kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains("call"));
}

#[test]
fn cranelift_codegen_compiles_bigint_arithmetic() {
    let backend = lower_text(
        "bigint-arith.aivi",
        r#"
fun add_bigints:BigInt = a:BigInt b:BigInt =>
    a + b
"#,
    );
    let body = backend.items()[find_item(&backend, "add_bigints")]
        .body
        .expect("should have body");
    let compiled = compile_program(&backend).expect("bigint arithmetic should compile");
    let artifact = compiled
        .kernel(body)
        .expect("compiled program should retain bigint add kernel metadata");
    assert!(artifact.code_size > 0);
    assert!(artifact.clif.contains("call"));
}

#[test]
fn cranelift_codegen_compiles_decimal_all_arithmetic_ops() {
    let ptr = clif_pointer_ty();
    let backend = lower_text(
        "decimal-all-arith.aivi",
        r#"
fun dec_add:Decimal = a:Decimal b:Decimal => a + b
fun dec_sub:Decimal = a:Decimal b:Decimal => a - b
fun dec_mul:Decimal = a:Decimal b:Decimal => a * b
fun dec_div:Decimal = a:Decimal b:Decimal => a / b
fun dec_mod:Decimal = a:Decimal b:Decimal => a % b
"#,
    );

    let compiled = compile_program(&backend).expect("all decimal arithmetic ops should compile");

    for name in &["dec_add", "dec_sub", "dec_mul", "dec_div", "dec_mod"] {
        let body = backend.items()[find_item(&backend, name)]
            .body
            .expect("should have body");
        let artifact = compiled
            .kernel(body)
            .unwrap_or_else(|| panic!("compiled program should retain {name} kernel metadata"));
        assert!(artifact.code_size > 0, "{name} should have non-zero code");
        assert!(
            artifact.clif.contains(&format!("({ptr}, {ptr}) -> {ptr}")),
            "{name} CLIF should reference (ptr, ptr) -> ptr signature"
        );
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_bigint_all_arithmetic_ops() {
    let ptr = clif_pointer_ty();
    let backend = lower_text(
        "bigint-all-arith.aivi",
        r#"
fun big_add:BigInt = a:BigInt b:BigInt => a + b
fun big_sub:BigInt = a:BigInt b:BigInt => a - b
fun big_mul:BigInt = a:BigInt b:BigInt => a * b
fun big_div:BigInt = a:BigInt b:BigInt => a / b
fun big_mod:BigInt = a:BigInt b:BigInt => a % b
"#,
    );

    let compiled = compile_program(&backend).expect("all bigint arithmetic ops should compile");

    for name in &["big_add", "big_sub", "big_mul", "big_div", "big_mod"] {
        let body = backend.items()[find_item(&backend, name)]
            .body
            .expect("should have body");
        let artifact = compiled
            .kernel(body)
            .unwrap_or_else(|| panic!("compiled program should retain {name} kernel metadata"));
        assert!(artifact.code_size > 0, "{name} should have non-zero code");
        assert!(
            artifact.clif.contains(&format!("({ptr}, {ptr}) -> {ptr}")),
            "{name} CLIF should reference (ptr, ptr) -> ptr signature"
        );
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_decimal_comparison() {
    let ptr = clif_pointer_ty();
    let backend = lower_text(
        "decimal-compare.aivi",
        r#"
fun dec_gt:Bool = a:Decimal b:Decimal => a > b
fun dec_lt:Bool = a:Decimal b:Decimal => a < b
fun dec_gte:Bool = a:Decimal b:Decimal => a >= b
fun dec_lte:Bool = a:Decimal b:Decimal => a <= b
fun dec_eq:Bool = a:Decimal b:Decimal => a == b
fun dec_ne:Bool = a:Decimal b:Decimal => a != b
"#,
    );

    let compiled = compile_program(&backend).expect("decimal comparison ops should compile");

    for name in &["dec_gt", "dec_lt", "dec_gte", "dec_lte", "dec_eq", "dec_ne"] {
        let body = backend.items()[find_item(&backend, name)]
            .body
            .expect("should have body");
        let artifact = compiled
            .kernel(body)
            .unwrap_or_else(|| panic!("compiled program should retain {name} kernel metadata"));
        assert!(artifact.code_size > 0, "{name} should have non-zero code");
        assert!(
            artifact.clif.contains(&format!("({ptr}, {ptr}) -> i8")),
            "{name} CLIF should reference (ptr, ptr) -> i8 comparison signature"
        );
    }
    assert!(!compiled.object().is_empty());
}

#[test]
fn cranelift_codegen_compiles_bigint_comparison() {
    let ptr = clif_pointer_ty();
    let backend = lower_text(
        "bigint-compare.aivi",
        r#"
fun big_gt:Bool = a:BigInt b:BigInt => a > b
fun big_lt:Bool = a:BigInt b:BigInt => a < b
fun big_gte:Bool = a:BigInt b:BigInt => a >= b
fun big_lte:Bool = a:BigInt b:BigInt => a <= b
fun big_eq:Bool = a:BigInt b:BigInt => a == b
fun big_ne:Bool = a:BigInt b:BigInt => a != b
"#,
    );

    let compiled = compile_program(&backend).expect("bigint comparison ops should compile");

    for name in &["big_gt", "big_lt", "big_gte", "big_lte", "big_eq", "big_ne"] {
        let body = backend.items()[find_item(&backend, name)]
            .body
            .expect("should have body");
        let artifact = compiled
            .kernel(body)
            .unwrap_or_else(|| panic!("compiled program should retain {name} kernel metadata"));
        assert!(artifact.code_size > 0, "{name} should have non-zero code");
        assert!(
            artifact.clif.contains(&format!("({ptr}, {ptr}) -> i8")),
            "{name} CLIF should reference (ptr, ptr) -> i8 comparison signature"
        );
    }
    assert!(!compiled.object().is_empty());
}

// Debug and fan-out codegen are implemented (no-op pass-through and loop emission
// respectively) but cannot be exercised from general-expression test fixtures:
// - Debug stages are only produced by @debug decorators on signal pipelines
// - Fan-out *|> in general expressions triggers SubjectLayoutMismatch at backend
//   lowering (pre-existing issue); signal-pipeline fan-out is tested by
//   retains_signal_fanout_map_and_join_kernels

// ── Pattern match codegen coverage tests ───────────────────────────────────────
