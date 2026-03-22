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
fn check_accepts_snake_fixture() {
    let relative = "milestone-2/valid/snake/main.aivi";
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
