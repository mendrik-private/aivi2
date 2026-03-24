use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
        .join(relative)
}

fn stdlib_path(relative: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib")
        .join(relative);
    fs::canonicalize(&path).unwrap_or(path)
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("aivi-{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("temporary directory should be creatable");
        Self { path }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("temporary parent directories should be creatable");
        }
        fs::write(&path, text).expect("temporary file should be writable");
        path
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn check_accepts_valid_hir_fixtures() {
    for relative in [
        "milestone-2/valid/local-top-level-refs/main.aivi",
        "milestone-2/valid/use-member-imports/main.aivi",
        "milestone-2/valid/use-member-import-aliases/main.aivi",
        "milestone-2/valid/workspace-type-imports/main.aivi",
        "milestone-2/valid/workspace-typeclass-prelude/main.aivi",
        "milestone-2/valid/foldable-reduce/main.aivi",
        "milestone-2/valid/source-provider-contract-declarations/main.aivi",
        "milestone-2/valid/custom-source-provider-wakeup/main.aivi",
        "milestone-2/valid/custom-source-recurrence-wakeup/main.aivi",
        "milestone-2/valid/map-set-literals/main.aivi",
        "milestone-2/valid/source-decorator-signals/main.aivi",
        "milestone-2/valid/source-option-contract-parameters/main.aivi",
        "milestone-2/valid/source-option-contract-parameter-context-free-builtins/main.aivi",
        "milestone-2/valid/source-option-imported-binding-match/main.aivi",
        "milestone-2/valid/applicative-clusters/main.aivi",
        "milestone-2/valid/builtin-constructor-inference/main.aivi",
        "milestone-2/valid/case-exhaustiveness/main.aivi",
        "milestone-2/valid/markup-control-nodes/main.aivi",
        "milestone-2/valid/class-declarations/main.aivi",
        "milestone-2/valid/instance-declarations/main.aivi",
        "milestone-2/valid/domain-declarations/main.aivi",
        "milestone-2/valid/domain-member-resolution/main.aivi",
        "milestone-2/valid/domain-literal-suffixes/main.aivi",
        "milestone-2/valid/noninteger-literals/main.aivi",
        "milestone-2/valid/domain-operator-usage/main.aivi",
        "milestone-2/valid/domain-operator-usage-parameterized/main.aivi",
        "milestone-2/valid/type-kinds/main.aivi",
        "milestone-2/valid/bundled-collection-stdlib/main.aivi",
        "milestone-2/valid/bundled-root-prelude-stdlib/main.aivi",
        "milestone-2/valid/pipe-branch-and-join/main.aivi",
        "milestone-2/valid/pipe-truthy-falsy-carriers/main.aivi",
        "milestone-2/valid/pipe-fanout-carriers/main.aivi",
        "milestone-2/valid/pipe-gate-carriers/main.aivi",
        "milestone-2/valid/pipe-recurrence-suffix/main.aivi",
        "milestone-2/valid/pipe-recurrence-nonsource-wakeup/main.aivi",
        "milestone-1/valid/records/record_shorthand_and_elision.aivi",
        "milestone-1/valid/sources/source_declarations.aivi",
        "milestone-1/valid/strings/text_and_regex.aivi",
        "milestone-1/valid/top-level/declarations.aivi",
        "milestone-1/valid/pipes/pipe_algebra.aivi",
    ] {
        let path = fixture_path(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("check command should run");

        assert!(
            output.status.success(),
            "expected {relative} to pass check, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
            "expected success output for {relative}, got stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn check_rejects_invalid_hir_fixtures() {
    for relative in [
        "milestone-2/invalid/duplicate-top-level-names/main.aivi",
        "milestone-2/invalid/duplicate-source-provider-contract/main.aivi",
        "milestone-2/invalid/duplicate-map-literal-key/main.aivi",
        "milestone-2/invalid/unknown-imported-names/main.aivi",
        "milestone-2/invalid/unknown-decorator/main.aivi",
        "milestone-2/invalid/unresolved-names/main.aivi",
        "milestone-2/invalid/misplaced-control-branches/main.aivi",
        "milestone-2/invalid/source-decorator-non-signal/main.aivi",
        "milestone-2/invalid/unknown-import-module/main.aivi",
        "milestone-2/invalid/domain-recursive-carrier/main.aivi",
        "milestone-2/invalid/ambiguous-domain-literal-suffix/main.aivi",
        "milestone-2/invalid/unpaired-truthy-falsy/main.aivi",
        "milestone-2/invalid/truthy-falsy-noncanonical-subject/main.aivi",
        "milestone-2/invalid/truthy-falsy-branch-type-mismatch/main.aivi",
        "milestone-2/invalid/truthy-falsy-payloadless-projection/main.aivi",
        "milestone-2/invalid/fanin-without-map/main.aivi",
        "milestone-2/invalid/fanout-non-list-subject/main.aivi",
        "milestone-2/invalid/fanin-invalid-projection/main.aivi",
        "milestone-2/invalid/gate-predicate-not-bool/main.aivi",
        "milestone-2/invalid/impure-gate-predicate/main.aivi",
        "milestone-2/invalid/cluster-ambient-projection/main.aivi",
        "milestone-2/invalid/orphan-recur-step/main.aivi",
        "milestone-2/invalid/unfinished-recurrence/main.aivi",
        "milestone-2/invalid/recurrence-continuation/main.aivi",
        "milestone-2/invalid/unknown-recurrence-target/main.aivi",
        "milestone-2/invalid/unsupported-recurrence-target/main.aivi",
        "milestone-2/invalid/missing-recurrence-wakeup/main.aivi",
        "milestone-2/invalid/custom-source-recurrence-missing-wakeup/main.aivi",
        "milestone-2/invalid/request-recurrence-missing-wakeup/main.aivi",
        "milestone-2/invalid/interpolated-pattern-text/main.aivi",
        "milestone-1/invalid/cluster_unfinished_gate.aivi",
        "milestone-1/invalid/pattern_non_exhaustive_sum.aivi",
        "milestone-1/invalid/regex_bad_pattern.aivi",
        "milestone-1/invalid/regex_invalid_quantifier.aivi",
        "milestone-1/invalid/source_unknown_option.aivi",
        "milestone-2/invalid/overapplied-type-constructor/main.aivi",
        "milestone-2/invalid/imported-overapplied-type-constructor/main.aivi",
        "milestone-2/invalid/underapplied-domain-constructor/main.aivi",
        "milestone-2/invalid/source-duplicate-option/main.aivi",
        "milestone-2/invalid/source-provider-without-variant/main.aivi",
        "milestone-2/invalid/source-legacy-quantity-option/main.aivi",
        "milestone-2/invalid/source-contract-missing-type/main.aivi",
        "milestone-2/invalid/source-contract-arity-mismatch/main.aivi",
        "milestone-2/invalid/source-option-type-mismatch/main.aivi",
        "milestone-2/invalid/source-option-contract-parameter-signal-mismatch/main.aivi",
        "milestone-2/invalid/source-option-unbound-contract-parameter/main.aivi",
        "milestone-2/invalid/source-option-imported-binding-mismatch/main.aivi",
        "milestone-2/invalid/source-option-constructor-mismatch/main.aivi",
        "milestone-2/invalid/source-option-list-element-mismatch/main.aivi",
        "milestone-2/invalid/value-annotation-type-mismatch/main.aivi",
        "milestone-2/invalid/noninteger-literal-type-mismatch/main.aivi",
        "milestone-2/invalid/equality-missing-eq-instance/main.aivi",
        "milestone-2/invalid/ambiguous-domain-member/main.aivi",
        "milestone-2/invalid/mixed-applicative-cluster/main.aivi",
        "milestone-2/invalid/case-branch-type-mismatch/main.aivi",
        "milestone-2/invalid/duplicate-instance/main.aivi",
        "milestone-2/invalid/instance-member-operator-mismatch/main.aivi",
        "milestone-2/invalid/operator-expression-typing/main.aivi",
        "milestone-2/invalid/trailing-declaration-body-token/main.aivi",
        "milestone-2/invalid/custom-source-provider-unknown-option/main.aivi",
        "milestone-2/invalid/custom-source-provider-option-type-mismatch/main.aivi",
        "milestone-2/invalid/custom-source-provider-argument-count-mismatch/main.aivi",
        "milestone-2/invalid/custom-source-provider-argument-type-mismatch/main.aivi",
        "milestone-2/invalid/custom-source-provider-unsupported-schema-type/main.aivi",
        "milestone-2/invalid/non-exhaustive-match-control/main.aivi",
    ] {
        let path = fixture_path(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("check command should run");

        assert!(
            !output.status.success(),
            "expected {relative} to fail check, stdout was: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !output.stderr.is_empty(),
            "expected diagnostics on stderr for {relative}"
        );
    }
}

#[test]
fn check_reports_regex_validation_from_hir() {
    let path = fixture_path("milestone-1/invalid/regex_invalid_quantifier.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected regex fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::invalid-regex-literal"),
        "expected HIR regex diagnostic, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("regex literal is not valid under the current compile-time regex grammar"),
        "expected explicit regex validation message, got stderr: {stderr}"
    );
    assert!(
        !stderr.contains("syntax::invalid-regex-literal"),
        "regex validation should no longer be reported from syntax, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_non_exhaustive_case_from_hir() {
    let path = fixture_path("milestone-1/invalid/pattern_non_exhaustive_sum.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected non-exhaustive case fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::non-exhaustive-case-pattern"),
        "expected HIR case exhaustiveness diagnostic, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("case split over `Status` is not exhaustive; missing `Pending`, `Failed`"),
        "expected explicit non-exhaustive case message, got stderr: {stderr}"
    );
}

#[test]
fn check_accepts_stdlib_foundation_validation_files() {
    for relative in [
        "aivi/nonEmpty.aivi",
        "aivi/validation.aivi",
        "tests/foundation-validation/main.aivi",
    ] {
        let path = stdlib_path(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("check command should run");

        assert!(
            output.status.success(),
            "expected {relative} to pass check, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
            "expected success output for {relative}, got stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn check_reports_unbound_source_option_contract_parameter_from_hir() {
    let path =
        fixture_path("milestone-2/invalid/source-option-unbound-contract-parameter/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected unbound source option fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::source-option-unbound-contract-parameter"),
        "expected unbound contract parameter diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains(
            "source option `body` for `http.post` expects `A`, but local source-option checking leaves contract parameter `A` unbound"
        ),
        "expected explicit unbound contract parameter message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_type_mismatch_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/value-annotation-type-mismatch/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected type mismatch fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::type-mismatch"),
        "expected type mismatch diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected `Text` but found `Int`"),
        "expected explicit type mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_noninteger_literal_type_mismatches_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/noninteger-literal-type-mismatch/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected noninteger literal mismatch fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::type-mismatch"),
        "expected type mismatch diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected `Float` but found `Decimal`"),
        "expected Float/Decimal mismatch message, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected `Decimal` but found `Float`"),
        "expected Decimal/Float mismatch message, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected `BigInt` but found `Int`"),
        "expected BigInt/Int mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_missing_eq_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/equality-missing-eq-instance/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected missing Eq fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::missing-eq-instance"),
        "expected missing Eq diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("this expression requires `Eq` for `Map Text Int`"),
        "expected explicit missing Eq message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_instance_member_operator_mismatch_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/instance-member-operator-mismatch/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected instance member operator mismatch fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::type-mismatch"),
        "expected type mismatch diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("expected `Bool` but found `Blob`"),
        "expected explicit instance member mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_invalid_operator_typing_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/operator-expression-typing/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected invalid operator fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::invalid-unary-operator"),
        "expected invalid unary operator diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("hir::invalid-binary-operator"),
        "expected invalid binary operator diagnostic code, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_trailing_body_tokens_from_syntax() {
    let path = fixture_path("milestone-2/invalid/trailing-declaration-body-token/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected trailing body token fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("syntax::trailing-declaration-body-token"),
        "expected trailing body token diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("function declaration body must contain exactly one expression"),
        "expected explicit trailing body token message, got stderr: {stderr}"
    );
}

#[test]
fn check_accepts_phase_one_collection_stdlib_modules() {
    for relative in [
        "aivi/list.aivi",
        "aivi/option.aivi",
        "aivi/result.aivi",
        "aivi/text.aivi",
    ] {
        let path = stdlib_path(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("check command should run");

        assert!(
            output.status.success(),
            "expected {relative} to pass check, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
            "expected success output for {relative}, got stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn check_accepts_phase_one_domain_stdlib_modules() {
    for relative in [
        "aivi/duration.aivi",
        "aivi/url.aivi",
        "aivi/path.aivi",
        "aivi/color.aivi",
    ] {
        let path = stdlib_path(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("check")
            .arg(&path)
            .output()
            .expect("check command should run");

        assert!(
            output.status.success(),
            "expected {relative} to pass check, stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
            "expected success output for {relative}, got stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn check_accepts_bundled_phase_one_domain_stdlib_imports() {
    let workspace = TempDir::new("check-domain-stdlib");
    workspace.write("aivi.toml", "");
    let main = workspace.write(
        "main.aivi",
        "use aivi.duration (\n    Duration\n    DurationError\n)\n\nuse aivi.url (\n    Url\n    UrlError\n)\n\nuse aivi.path (\n    Path\n    PathError\n)\n\nuse aivi.color (\n    Color\n)\n\ntype Delay = Duration\ntype DelayFailure = DurationError\ntype Endpoint = Url\ntype EndpointFailure = UrlError\ntype FilePath = Path\ntype FilePathFailure = PathError\ntype ThemeColor = Color\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&main)
        .current_dir(workspace.path())
        .output()
        .expect("check command should run");

    assert!(
        output.status.success(),
        "expected bundled phase-one domain imports to pass check, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
        "expected success output, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn check_accepts_bundled_stdlib_fallback() {
    let workspace = TempDir::new("check-bundled-stdlib");
    workspace.write("aivi.toml", "");
    let main = workspace.write(
        "main.aivi",
        "use aivi.bundledsmoketest (\n    bundledSentinel\n    BundledToken\n)\n\ntype Alias = BundledToken\nval marker = bundledSentinel\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&main)
        .current_dir(workspace.path())
        .output()
        .expect("check command should run");

    assert!(
        output.status.success(),
        "expected bundled stdlib fallback to pass check, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
        "expected success output, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn check_accepts_bundled_root_and_prelude_stdlib_imports() {
    let workspace = TempDir::new("check-root-prelude-stdlib");
    workspace.write("aivi.toml", "");
    let main = workspace.write(
        "main.aivi",
        "use aivi (\n    Option\n    Result\n    Validation\n    Signal\n    Task\n    Some\n    None\n    Ok\n    Err\n    Valid\n    Invalid\n)\n\nuse aivi.prelude (\n    Int\n    Bool\n    Text\n    List\n    Eq\n    Default\n    Functor\n    Applicative\n    Monad\n    Foldable\n    getOrElse\n    withDefault\n    length\n    head\n    join\n)\n\ntype NameSignal = Signal Text\ntype CountTask = Task Text Int\ntype CheckedName = Validation Text Text\n\nval maybeName:Option Text = Some \"Ada\"\nval missingName:Option Text = None\nval chosenName:Text = getOrElse \"guest\" missingName\n\nval okCount:Result Text Int = Ok 2\nval errCount:Result Text Int = Err \"missing\"\nval chosenCount:Int = withDefault 0 okCount\n\nval checkedName:CheckedName = Valid \"Ada\"\nval nameCount:Int = length [\"Ada\", \"Grace\"]\nval firstName:Option Text = head [\"Ada\", \"Grace\"]\nval labels:Text = join \", \" [\"Ada\", \"Grace\"]\nval sameCount:Bool = chosenCount == 2\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&main)
        .current_dir(workspace.path())
        .output()
        .expect("check command should run");

    assert!(
        output.status.success(),
        "expected bundled root/prelude imports to pass check, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("syntax + HIR passed"),
        "expected success output, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn check_reports_ambiguous_domain_members_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/ambiguous-domain-member/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected ambiguous domain member fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::ambiguous-domain-member"),
        "expected ambiguous domain member diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("domain member `make` is ambiguous in this context"),
        "expected explicit ambiguous domain member message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_mixed_applicative_clusters_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/mixed-applicative-cluster/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected mixed applicative cluster fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::applicative-cluster-mismatch"),
        "expected applicative cluster mismatch diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("`&|>` cluster mixes `Option _` with `Signal _`"),
        "expected explicit applicative cluster mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn check_reports_case_branch_type_mismatch_from_hir_typechecker() {
    let path = fixture_path("milestone-2/invalid/case-branch-type-mismatch/main.aivi");
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check command should run");

    assert!(
        !output.status.success(),
        "expected case branch type mismatch fixture to fail check"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hir::case-branch-type-mismatch"),
        "expected case branch type mismatch diagnostic code, got stderr: {stderr}"
    );
    assert!(
        stderr
            .contains("case split branches must agree on one result type, found `Int` and `Text`"),
        "expected explicit case branch type mismatch message, got stderr: {stderr}"
    );
}
