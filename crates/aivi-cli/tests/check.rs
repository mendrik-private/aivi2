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
        "milestone-2/valid/source-decorator-signals/main.aivi",
        "milestone-2/valid/applicative-clusters/main.aivi",
        "milestone-2/valid/markup-control-nodes/main.aivi",
        "milestone-2/valid/class-declarations/main.aivi",
        "milestone-2/valid/domain-declarations/main.aivi",
        "milestone-2/valid/domain-literal-suffixes/main.aivi",
        "milestone-2/valid/type-kinds/main.aivi",
        "milestone-2/valid/pipe-branch-and-join/main.aivi",
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
        "milestone-2/invalid/unknown-imported-names/main.aivi",
        "milestone-2/invalid/unknown-decorator/main.aivi",
        "milestone-2/invalid/unresolved-names/main.aivi",
        "milestone-2/invalid/misplaced-control-branches/main.aivi",
        "milestone-2/invalid/source-decorator-non-signal/main.aivi",
        "milestone-2/invalid/unknown-import-module/main.aivi",
        "milestone-2/invalid/domain-recursive-carrier/main.aivi",
        "milestone-2/invalid/ambiguous-domain-literal-suffix/main.aivi",
        "milestone-2/invalid/unpaired-truthy-falsy/main.aivi",
        "milestone-2/invalid/fanin-without-map/main.aivi",
        "milestone-2/invalid/interpolated-pattern-text/main.aivi",
        "milestone-1/invalid/cluster_unfinished_gate.aivi",
        "milestone-1/invalid/source_unknown_option.aivi",
        "milestone-2/invalid/overapplied-type-constructor/main.aivi",
        "milestone-2/invalid/underapplied-domain-constructor/main.aivi",
        "milestone-2/invalid/source-duplicate-option/main.aivi",
        "milestone-2/invalid/source-provider-without-variant/main.aivi",
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
