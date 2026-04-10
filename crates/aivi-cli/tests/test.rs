use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn stdlib_path(relative: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib")
        .join(relative);
    fs::canonicalize(&path).unwrap_or(path)
}

#[test]
fn test_command_accepts_stdlib_validation_files() {
    for (relative, summary) in [
        (
            "aivi/text.aivi",
            "test result: ok. 3 passed; 0 failed; 3 total",
        ),
        (
            "aivi/math.aivi",
            "test result: ok. 2 passed; 0 failed; 2 total",
        ),
        (
            "aivi/bool.aivi",
            "test result: ok. 2 passed; 0 failed; 2 total",
        ),
        (
            "aivi/defaults.aivi",
            "test result: ok. 3 passed; 0 failed; 3 total",
        ),
        (
            "aivi/core/float.aivi",
            "test result: ok. 2 passed; 0 failed; 2 total",
        ),
        (
            "tests/runtime-stdlib-validation/main.aivi",
            "test result: ok. 3 passed; 0 failed; 3 total",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
            .arg("test")
            .arg(stdlib_path(relative))
            .output()
            .expect("test command should run");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "expected {relative} to pass `aivi test`, stdout was: {stdout}, stderr was: {stderr}"
        );
        assert!(
            stderr.is_empty(),
            "expected {relative} to keep stderr empty, stderr was: {stderr}"
        );
        assert!(
            stdout.contains(summary),
            "expected success summary for {relative}, stdout was: {stdout}"
        );
    }
}

#[test]
fn test_command_reports_when_workspace_has_no_tests() {
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("test")
        .arg(stdlib_path("aivi/order.aivi"))
        .output()
        .expect("test command should run");

    assert!(
        !output.status.success(),
        "expected `aivi test` to fail when no `@test` values exist"
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no `@test` values found in the loaded workspace"),
        "expected missing-test diagnostic, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_command_accepts_contains_predicate_forms() {
    let dir = TempDir::new("test-contains-predicate-forms");
    let path = dir.write(
        "main.aivi",
        concat!(
            "use aivi.list (contains)\n",
            "type Coord = Coord Int Int\n",
            "type Coord -> Coord -> Bool\n",
            "func coordEq = target candidate => candidate == target\n",
            "type Coord -> List Coord -> Bool\n",
            "func member = cell items => contains (coordEq cell) items\n",
            "value items : List Coord = [Coord 0 0, Coord 1 1]\n",
            "value cell : Coord = Coord 1 1\n",
            "value directMatch : Bool = contains (. == cell) items\n",
            "value helperMatch : Bool = member cell items\n",
            "value missingMatch : Bool = not (contains (. == (Coord 9 9)) items)\n",
            "@test\n",
            "value directContains : Task Text Bool = pure directMatch\n",
            "@test\n",
            "value helperContains : Task Text Bool = pure helperMatch\n",
            "@test\n",
            "value missingContains : Task Text Bool = pure missingMatch\n",
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("test")
        .arg(&path)
        .output()
        .expect("test command should run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected contains predicate forms to pass `aivi test`, stdout was: {stdout}, stderr was: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "expected contains predicate forms to keep stderr empty, stderr was: {stderr}"
    );
    assert!(
        stdout.contains("test result: ok. 3 passed; 0 failed; 3 total"),
        "expected success summary for contains predicate forms, stdout was: {stdout}"
    );
}
