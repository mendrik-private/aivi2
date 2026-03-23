use std::{env, path::PathBuf, process::Command};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
        .join(relative)
}

#[test]
fn check_accepts_valid_hir_fixtures() {
    for relative in [
        "milestone-2/valid/local-top-level-refs/main.aivi",
        "milestone-2/valid/use-member-imports/main.aivi",
        "milestone-2/valid/use-member-import-aliases/main.aivi",
        "milestone-2/valid/source-provider-contract-declarations/main.aivi",
        "milestone-2/valid/custom-source-provider-wakeup/main.aivi",
        "milestone-2/valid/custom-source-recurrence-wakeup/main.aivi",
        "milestone-2/valid/map-set-literals/main.aivi",
        "milestone-2/valid/source-decorator-signals/main.aivi",
        "milestone-2/valid/source-option-contract-parameters/main.aivi",
        "milestone-2/valid/source-option-contract-parameter-context-free-builtins/main.aivi",
        "milestone-2/valid/source-option-imported-binding-match/main.aivi",
        "milestone-2/valid/applicative-clusters/main.aivi",
        "milestone-2/valid/case-exhaustiveness/main.aivi",
        "milestone-2/valid/markup-control-nodes/main.aivi",
        "milestone-2/valid/class-declarations/main.aivi",
        "milestone-2/valid/domain-declarations/main.aivi",
        "milestone-2/valid/domain-member-resolution/main.aivi",
        "milestone-2/valid/domain-literal-suffixes/main.aivi",
        "milestone-2/valid/type-kinds/main.aivi",
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
        "milestone-2/invalid/source-option-imported-binding-mismatch/main.aivi",
        "milestone-2/invalid/source-option-constructor-mismatch/main.aivi",
        "milestone-2/invalid/source-option-list-element-mismatch/main.aivi",
        "milestone-2/invalid/value-annotation-type-mismatch/main.aivi",
        "milestone-2/invalid/equality-missing-eq-instance/main.aivi",
        "milestone-2/invalid/ambiguous-domain-member/main.aivi",
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
